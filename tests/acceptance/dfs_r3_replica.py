#!/usr/bin/env python3
"""Read-only DFS R3 replica proof without moving chunk payloads.

Modes:
  meta     Run on ctl. Decode local-file Meta snapshot/WAL and emit a compact
           expected CopyRecord manifest.
  node     Run on each data node. Read that manifest, verify local catalog rows
           and physical chunk bytes in the original data_dir/dfs tree, and emit
           a compact node report.
  combine  Run on ctl. Combine the Meta manifest with three node reports into
           the final proof JSON.

The helper never starts/stops services and never persists source snapshots. It
proves persisted CopyRecord Ready/Durable facts plus local physical bytes; it does
not claim complete original ReplicaAck persistence. Live authority is observed
at the ctl clock when reading committed Meta state; historical receipts are
retained and only distinct eligible serving nodes count.
"""
import argparse
import hashlib
import json
import os
import struct
import sys
import time
import stat
import traceback
from pathlib import Path

FRAME_MAGIC = b"AFSL"
FRAME_HEADER_LEN = 28
EXPECTED_BYTES = 64 * 1024 * 1024
EXPECTED_CHUNK_BYTES = 4 * 1024 * 1024
EXPECTED_BLOCK_BYTES = 1024 * 1024
EXPECTED_CHUNKS = 16
EXPECTED_COPIES = 3
EXPECTED_SHA256 = "4c47e859a6831026ba9367262afb1e1803d11abc30f6f3d996ceea829af89324"
DATASET = "counter-1m-v1"
SOURCE_COMMIT = "f03dc2b3679c31daa51caee275fb2087413e949c"
NODE_ELF_SHA256 = "3b1f1dce187a6285814c03b9024cdfc5f2dec990a73cba9cc13b3dc9ef402d36"
META_ELF_SHA256 = "c7447bcfac7e3f8bf605446ade5e11be74f1333709a8f7bb5378b1d6ee7506fd"


def fail(message):
    raise ValueError(message)


def sha256_file(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def base_result(mode):
    return {
        "status": "FAIL",
        "mode": mode,
        "dataset": DATASET,
        "source_commit": SOURCE_COMMIT,
        "meta_elf_sha256": META_ELF_SHA256,
        "node_elf_sha256": NODE_ELF_SHA256,
        "observer_sha256": sha256_file(__file__),
        "limits": {
            "read_only": True,
            "expected_bytes": EXPECTED_BYTES,
            "expected_chunk_bytes": EXPECTED_CHUNK_BYTES,
            "expected_chunks": EXPECTED_CHUNKS,
            "expected_copies_per_chunk": EXPECTED_COPIES,
            "does_not_claim_original_replica_ack_persistence": True,
            "does_not_move_chunk_payloads": True,
        },
    }


def write_result(result, out):
    text = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if out:
        Path(out).write_text(text)
    print(text, end="")
    return 0 if result["status"] == "PASS" else 1


def run_guard():
    if os.name != "posix" or not Path("/proc").exists():
        fail("Linux runtime required")


def afsl_checksum(version, length, payload):
    h = 0xCBF29CE484222325
    for byte in FRAME_MAGIC + struct.pack("<Q", version) + struct.pack("<Q", length) + payload:
        h ^= byte
        h = (h * 0x00000100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def decode_frames(path, reject_tail):
    path = Path(path)
    if not path.exists():
        return []
    data = path.read_bytes()
    frames = []
    offset = 0
    while offset < len(data):
        if len(data) - offset < FRAME_HEADER_LEN:
            if reject_tail:
                fail(f"{path}: incomplete frame header at {offset}")
            break
        header = data[offset:offset + FRAME_HEADER_LEN]
        if header[:4] != FRAME_MAGIC:
            fail(f"{path}: frame magic mismatch at {offset}")
        version, length, want = struct.unpack("<QQQ", header[4:28])
        start = offset + FRAME_HEADER_LEN
        end = start + length
        if end > len(data):
            if reject_tail:
                fail(f"{path}: incomplete frame payload at {offset}")
            break
        payload = data[start:end]
        got = afsl_checksum(version, length, payload)
        if got != want:
            fail(f"{path}: frame checksum mismatch at {offset}")
        frames.append({"version": version, "payload": payload, "offset": offset, "end": end})
        offset = end
    return frames


def load_meta_state(meta_store_dir):
    root = Path(meta_store_dir)
    state = None
    base_version = 0
    snapshot_frames = decode_frames(root / "snapshot", reject_tail=True)
    if snapshot_frames:
        if len(snapshot_frames) != 1:
            fail("snapshot must contain exactly one frame")
        state = snapshot_frames[0]
        base_version = state["version"]
    current = base_version
    wal_frames = []
    for frame in decode_frames(root / "wal", reject_tail=False):
        wal_frames.append({k: v for k, v in frame.items() if k != "payload"})
        if frame["version"] > current:
            if frame["version"] != current + 1:
                fail(f"wal version gap: expected {current + 1}, found {frame['version']}")
            current = frame["version"]
            state = frame
    if state is None:
        fail("no committed local-file Meta frame found")
    payload = json.loads(state["payload"].decode("utf-8"))
    return {
        "dir": str(root),
        "version": state["version"],
        "snapshot_frames": len(snapshot_frames),
        "wal_frames": len(wal_frames),
        "payload": payload,
    }


def variant_payload(value, name):
    if isinstance(value, dict) and list(value.keys()) == [name]:
        return value[name]
    return None


def unwrap_entity(entity):
    if not isinstance(entity, dict) or len(entity) != 1:
        fail(f"unexpected MetaEntity shape: {entity!r}")
    return next(iter(entity.items()))


def entity_maps(state):
    entities = []
    for pair in state.get("entities", []):
        if not isinstance(pair, list) or len(pair) != 2:
            fail("persisted entity entry is not a [key, value] pair")
        key, versioned = pair
        ent_name, ent = unwrap_entity(versioned["entity"])
        entities.append({"key": key, "revision": versioned["revision"], "kind": ent_name, "entity": ent})
    by_kind = {}
    for item in entities:
        by_kind.setdefault(item["kind"], []).append(item)
    return entities, by_kind


def read_catalog(chunk_root):
    root = Path(chunk_root)
    catalog = {}
    revisions = []
    path = root / "catalog.wal"
    if path.exists():
        with path.open("r", encoding="utf-8") as f:
            for lineno, line in enumerate(f, 1):
                if not line.strip():
                    continue
                txn = json.loads(line)
                revisions.append(txn["revision"])
                if txn["revision"] != len(revisions):
                    fail(f"{path}: non-contiguous catalog revision at line {lineno}")
                for record in txn.get("records", []):
                    cid = record["chunk"]["id"]
                    if record.get("catalog_revision") != txn["revision"]:
                        fail(f"{path}: record revision mismatch for {cid}")
                    catalog[cid] = record
    epoch_path = root / "device_epoch"
    if epoch_path.is_symlink():
        fail("device epoch path is a symlink")
    epoch = int(epoch_path.read_text())
    if epoch <= 0:
        fail("device epoch must be positive")
    return {"root": str(root), "catalog_wal": str(path), "revisions": revisions, "records": catalog, "device_epoch": epoch}


def content_digest_hex(obj):
    raw = obj["content_digest"]["bytes"]
    if isinstance(raw, list):
        return "".join(f"{int(b):02x}" for b in raw)
    if isinstance(raw, str):
        return raw
    fail("unsupported content digest byte shape")


def expected_chunk_bytes(chunk_index):
    out = bytearray()
    first_block = chunk_index * (EXPECTED_CHUNK_BYTES // EXPECTED_BLOCK_BYTES)
    for block in range(first_block, first_block + EXPECTED_CHUNK_BYTES // EXPECTED_BLOCK_BYTES):
        block_bytes = bytearray([0x61]) * EXPECTED_BLOCK_BYTES
        block_bytes[:8] = struct.pack("<Q", block)
        out.extend(block_bytes)
    return bytes(out)


def expected_chunk_sha256(chunk_index):
    return hashlib.sha256(expected_chunk_bytes(chunk_index)).hexdigest()


def oracle_file_sha256():
    h = hashlib.sha256()
    for index in range(EXPECTED_CHUNKS):
        h.update(expected_chunk_bytes(index))
    return h.hexdigest()


def verify_physical_chunk(path, chunk, chunk_index):
    path = Path(path)
    st = path.lstat()
    if not stat.S_ISREG(st.st_mode):
        fail(f"not a regular file: {path}")
    if st.st_size != chunk["length"]:
        fail(f"wrong physical length for {path}: {st.st_size} != {chunk['length']}")
    data = path.read_bytes()
    expected = expected_chunk_bytes(chunk_index)
    if data != expected:
        fail(f"content mismatch for {path}")
    return {"path": str(path), "bytes": st.st_size, "sha256": hashlib.sha256(data).hexdigest(), "dev": st.st_dev, "ino": st.st_ino}


def copy_node_location(copy):
    return variant_payload(copy.get("location"), "Node")


def current_sessions(by_kind):
    sessions = {}
    for item in by_kind.get("NodeSession", []):
        pointer = variant_payload(item["key"], "CurrentNodeSession")
        if pointer is None:
            continue
        node = pointer["node_id"]
        session = item["entity"]
        if node != session.get("node_id") or node in sessions:
            fail("inconsistent or duplicate current Node session pointer")
        sessions[node] = session
    return sessions


def classify_receipt(copy_id, copy, chunk, sessions, now):
    loc = copy_node_location(copy) if copy else None
    result = {"copy_id": copy_id, "record": copy, "location": loc, "eligible": False}
    if not copy or copy.get("id") != copy_id or not loc:
        return result | {"reason": "missing or non-Node receipt"}
    if (copy.get("chunk_id") != chunk["id"] or copy.get("role") != "DurableReplica"
            or copy.get("state") != "Ready" or copy.get("persisted_bytes") != chunk["length"]
            or copy.get("verified_digest") != chunk["content_digest"]):
        return result | {"reason": "not Ready Durable / matching full chunk identity"}
    session = sessions.get(loc["node_id"])
    if (not session or loc["node_epoch"] == 0 or session["lease_epoch"] < loc["node_epoch"]
            or session["expires_at_unix_ms"] <= now):
        return result | {"reason": "missing, expired or regressed current authority"}
    device = next((device for device in session["storage_devices"]
        if device["device_id"] == loc["device_id"] and device["device_epoch"] == loc["device_epoch"]
        and (session["lease_epoch"] == loc["node_epoch"]
             or device["catalog_revision"] >= loc["catalog_revision"])), None)
    if device is None:
        return result | {"reason": "changed device or insufficient cross-epoch catalog"}
    return result | {"eligible": True, "reason": "current serving authority",
                     "serving_location": loc | {"node_epoch": session["lease_epoch"]},
                     "session": session}


def select_serving_copies(placement, copies, chunk, sessions, now):
    historical = [classify_receipt(key, copies.get(key), chunk, sessions, now)
                  for key in placement.get("copies", [])]
    by_node = {}
    for receipt in historical:
        if receipt["eligible"]:
            by_node.setdefault(receipt["location"]["node_id"], []).append(receipt)
    if len(by_node) != EXPECTED_COPIES:
        fail(f"expected {EXPECTED_COPIES} distinct current serving nodes for {chunk['id']}, found {len(by_node)}")
    # One physical proof per node, preferring its latest receipt. All receipts,
    # including eligible historical alternatives, remain in the manifest.
    selected = [max(items, key=lambda item: (item["location"]["node_epoch"],
                    item["location"]["catalog_revision"], item["copy_id"]))
                for _, items in sorted(by_node.items())]
    return selected, historical


def verify_catalog_record(catalog, chunk, loc):
    cid = chunk["chunk"]["id"]
    record = catalog["records"].get(cid)
    tip = catalog["revisions"][-1] if catalog["revisions"] else 0
    if (record is None or record.get("state") != "Durable" or record.get("encoding") != "Raw"
            or record.get("chunk") != chunk["chunk"] or record.get("stored_length") != chunk["chunk"]["length"]
            or record.get("stored_checksum") != chunk["chunk"]["content_digest"]):
        fail(f"local Durable Raw record does not match complete chunk identity for {cid}")
    if (record.get("device_id") != loc["device_id"] or record.get("device_epoch") != loc["device_epoch"]
            or catalog["device_epoch"] != loc["device_epoch"] or tip < loc["catalog_revision"]
            or not 0 < record["catalog_revision"] <= tip):
        fail(f"local device/catalog does not cover receipt floor for {cid}")
    return record, tip


def build_manifest(meta_store, expected_sha256):
    meta = load_meta_state(meta_store)
    _, by_kind = entity_maps(meta["payload"])
    authority_at_unix_ms = time.time_ns() // 1_000_000
    sessions = current_sessions(by_kind)
    schema_version = meta["payload"].get("schema_version")
    if schema_version != 1:
        fail("unexpected Meta snapshot schema_version")
    chunks = {item["entity"]["id"]: item["entity"] for item in by_kind.get("DfsChunk", [])}
    placements = {item["entity"]["chunk_id"]: item["entity"] for item in by_kind.get("DfsPlacement", [])}
    copies = {item["entity"]["id"]: item["entity"] for item in by_kind.get("DfsCopy", [])}
    layouts = [item["entity"] for item in by_kind.get("DfsLayoutRoot", []) if item["entity"].get("file_length") == EXPECTED_BYTES]
    versions = [item["entity"] for item in by_kind.get("DfsFileVersion", []) if item["entity"].get("length") == EXPECTED_BYTES]
    if len(layouts) != 1:
        fail(f"expected exactly one {EXPECTED_BYTES}-byte DfsLayoutRoot, found {len(layouts)}")
    if len(versions) != 1:
        fail(f"expected exactly one {EXPECTED_BYTES}-byte DfsFileVersion, found {len(versions)}")
    layout = layouts[0]
    version = versions[0]
    if version.get("layout_root") != layout.get("id"):
        fail("FileVersion does not reference the selected LayoutRoot")
    extents = sorted(layout.get("inline_extents", []), key=lambda e: e["file_offset"])
    if len(extents) != EXPECTED_CHUNKS:
        fail(f"expected {EXPECTED_CHUNKS} extents, found {len(extents)}")
    if oracle_file_sha256() != expected_sha256:
        fail("local counter-1m-v1 oracle SHA does not match expected")

    chunk_manifest = []
    seen = set()
    for index, extent in enumerate(extents):
        if extent["file_offset"] != index * EXPECTED_CHUNK_BYTES or extent["length"] != EXPECTED_CHUNK_BYTES or extent["chunk_offset"] != 0:
            fail(f"unexpected extent layout at index {index}: {extent}")
        cid = extent["chunk_id"]
        if cid in seen:
            fail(f"duplicate chunk id in layout: {cid}")
        seen.add(cid)
        chunk = chunks.get(cid)
        if chunk is None:
            fail(f"missing DfsChunk for {cid}")
        if chunk.get("length") != EXPECTED_CHUNK_BYTES or chunk.get("encoding") != "Raw":
            fail(f"unexpected DfsChunk length/encoding for {cid}")
        digest_hex = content_digest_hex(chunk)
        if chunk.get("id") != f"blake3-{digest_hex}-{EXPECTED_CHUNK_BYTES}":
            fail(f"ChunkId does not match persisted digest/length for {cid}")
        placement = placements.get(cid)
        if placement is None:
            fail(f"missing DfsPlacement for {cid}")
        ready, historical = select_serving_copies(placement, copies, chunk, sessions, authority_at_unix_ms)
        chunk_manifest.append({
            "index": index,
            "file_offset": extent["file_offset"],
            "chunk_id": cid,
            "chunk": chunk,
            "content_digest": digest_hex,
            "expected_sha256": expected_chunk_sha256(index),
            "ready_durable_copies": ready,
            "historical_receipts": historical,
        })
    if len({item["content_digest"] for item in chunk_manifest}) != EXPECTED_CHUNKS:
        fail("layout does not contain 16 distinct 4MiB chunk contents")
    return {
        "status": "PASS",
        "dataset": DATASET,
        "source_commit": SOURCE_COMMIT,
        "meta_elf_sha256": META_ELF_SHA256,
        "node_elf_sha256": NODE_ELF_SHA256,
        "file_bytes": EXPECTED_BYTES,
        "file_sha256": expected_sha256,
        "authority_at_unix_ms": authority_at_unix_ms,
        "current_sessions": sessions,
        "file_version_id": version.get("id"),
        "layout_root_id": layout.get("id"),
        "meta": {k: v for k, v in meta.items() if k != "payload"} | {"schema_version": schema_version},
        "chunks": chunk_manifest,
        "limits": {
            "read_only": True,
            "expected_chunk_bytes": EXPECTED_CHUNK_BYTES,
            "expected_chunks": EXPECTED_CHUNKS,
            "expected_copies_per_chunk": EXPECTED_COPIES,
            "does_not_claim_original_replica_ack_persistence": True,
        },
    }


def load_manifest(path):
    data = json.loads(Path(path).read_text())
    if data.get("status") != "PASS" or data.get("dataset") != DATASET or data.get("file_sha256") != EXPECTED_SHA256 or data.get("source_commit") != SOURCE_COMMIT or data.get("meta_elf_sha256") != META_ELF_SHA256 or data.get("node_elf_sha256") != NODE_ELF_SHA256 or data.get("mode") != "meta" or data.get("authority_at_unix_ms", 0) <= 0:
        fail("manifest identity/status mismatch")
    return data


def mode_meta(args):
    result = base_result("meta")
    try:
        run_guard()
        manifest = build_manifest(args.meta_store, args.expected_sha256)
        result.update(manifest)
        result["observer_sha256"] = sha256_file(__file__)
    except Exception as exc:
        result.update({"error": repr(exc), "traceback": traceback.format_exc()})
    return write_result(result, args.out)


def mode_node(args):
    result = base_result("node")
    try:
        run_guard()
        manifest = load_manifest(args.manifest)
        catalog = read_catalog(args.chunk_root)
        verified = []
        for chunk in manifest["chunks"]:
            cid = chunk["chunk_id"]
            expected = [copy for copy in chunk["ready_durable_copies"] if copy["location"]["node_id"] == args.node_id]
            if not expected:
                continue
            if len(expected) != 1:
                fail(f"manifest has duplicate copy for node {args.node_id} chunk {cid}")
            loc = expected[0]["location"]
            record, tip = verify_catalog_record(catalog, chunk, loc)
            rel = variant_payload(record.get("location"), "PerChunkFile")
            if not rel or rel.get("relative_path") != f"chunks/{cid}":
                fail(f"node {args.node_id} record has non-canonical path for {cid}")
            physical = Path(catalog["root"]) / rel["relative_path"]
            if not physical.resolve().is_relative_to(Path(catalog["root"]).resolve()):
                fail("physical chunk path escapes local catalog root")
            proof = verify_physical_chunk(physical, chunk["chunk"], chunk["index"])
            if proof["sha256"] != chunk["expected_sha256"]:
                fail(f"node {args.node_id} physical SHA mismatch for {cid}")
            proof.update({
                "chunk_index": chunk["index"],
                "chunk_id": cid,
                "copy_id": expected[0]["copy_id"],
                "node_id": args.node_id,
                "catalog_revision": record.get("catalog_revision"),
                "catalog_tip_revision": tip,
                "receipt_catalog_floor": loc["catalog_revision"],
                "serving_node_epoch": expected[0]["serving_location"]["node_epoch"],
                "device_id": record.get("device_id"),
                "device_epoch": record.get("device_epoch"),
                "content_digest": chunk["content_digest"],
            })
            verified.append(proof)
        if len(verified) != EXPECTED_CHUNKS:
            fail(f"node {args.node_id} verified {len(verified)} chunks, expected {EXPECTED_CHUNKS}")
        result.update({
            "status": "PASS",
            "content_ok": True,
            "node_id": args.node_id,
            "manifest_sha256": sha256_file(args.manifest),
            "chunk_root": str(Path(args.chunk_root)),
            "catalog": {"catalog_wal": catalog["catalog_wal"], "revision": catalog["revisions"][-1] if catalog["revisions"] else 0, "records": len(catalog["records"])},
            "verified_chunks": verified,
        })
    except Exception as exc:
        result.update({"error": repr(exc), "traceback": traceback.format_exc()})
    return write_result(result, args.out)


def verify_physical_receipt(item, chunk, receipt):
    loc = receipt["location"]
    if (item["node_id"] != loc["node_id"] or item["copy_id"] != receipt["copy_id"]
            or item["chunk_id"] != chunk["chunk_id"] or item["sha256"] != chunk["expected_sha256"]
            or item["bytes"] != EXPECTED_CHUNK_BYTES
            or item["receipt_catalog_floor"] != loc["catalog_revision"]
            or item["catalog_tip_revision"] < loc["catalog_revision"]
            or not 0 < item["catalog_revision"] <= item["catalog_tip_revision"]
            or item["device_epoch"] != loc["device_epoch"] or item["device_id"] != loc["device_id"]
            or item["serving_node_epoch"] != receipt["serving_location"]["node_epoch"]):
        fail("physical proof does not match selected receipt identity / catalog floor")


def mode_combine(args):
    result = base_result("combine")
    try:
        run_guard()
        manifest = load_manifest(args.manifest)
        reports = [json.loads(Path(path).read_text()) for path in args.node_report]
        if len(reports) != EXPECTED_COPIES:
            fail(f"expected {EXPECTED_COPIES} node reports, found {len(reports)}")
        seen_nodes = {report.get("node_id") for report in reports}
        if len(seen_nodes) != EXPECTED_COPIES:
            fail("node reports are not from distinct nodes")
        manifest_sha = sha256_file(args.manifest)
        report_by_key = {}
        node_catalogs = {}
        for report in reports:
            if report.get("status") != "PASS" or report.get("mode") != "node" or report.get("dataset") != DATASET or report.get("manifest_sha256") != manifest_sha:
                fail(f"bad node report identity for {report.get('node_id')}")
            node_catalogs[report["node_id"]] = report["catalog"] | {"chunk_root": report["chunk_root"]}
            for item in report.get("verified_chunks", []):
                report_by_key[(item["node_id"], item["chunk_id"], item["copy_id"])] = item
        chunks = []
        for chunk in manifest["chunks"]:
            physical = []
            for copy in chunk["ready_durable_copies"]:
                loc = copy["location"]
                key = (loc["node_id"], chunk["chunk_id"], copy["copy_id"])
                item = report_by_key.get(key)
                if item is None:
                    fail(f"missing physical proof for {key}")
                verify_physical_receipt(item, chunk, copy)
                physical.append(item)
            if len({item["node_id"] for item in physical}) != EXPECTED_COPIES:
                fail(f"chunk {chunk['chunk_id']} does not have {EXPECTED_COPIES} node proofs")
            chunks.append({
                "index": chunk["index"],
                "file_offset": chunk["file_offset"],
                "chunk_id": chunk["chunk_id"],
                "chunk_bytes": chunk["chunk"]["length"],
                "content_digest": chunk["content_digest"],
                "expected_sha256": chunk["expected_sha256"],
                "ready_durable_copies": EXPECTED_COPIES,
                "ready_nodes": sorted(item["node_id"] for item in physical),
                "physical": sorted(physical, key=lambda item: item["node_id"]),
            })
        if oracle_file_sha256() != manifest["file_sha256"]:
            fail("counter oracle SHA changed during combine")
        result.update({
            "status": "PASS",
            "content_ok": True,
            "file_bytes": manifest["file_bytes"],
            "file_sha256": manifest["file_sha256"],
            "manifest_sha256": manifest_sha,
            "layout_root_id": manifest["layout_root_id"],
            "file_version_id": manifest["file_version_id"],
            "meta": manifest["meta"],
            "authority_at_unix_ms": manifest["authority_at_unix_ms"],
            "current_sessions": manifest["current_sessions"],
            "node_catalogs": node_catalogs,
            "chunks": chunks,
            "invariant": "one committed 64MiB FileVersion -> 16 unique 4MiB DfsChunks -> each chunk has exactly three distinct current serving nodes backed by eligible Ready Durable receipts (all historical records retained) -> each selected receipt matches a durable local catalog record and exact physical bytes verified in place on its source node; reconstructed oracle SHA256 matches counter-1m-v1",
        })
    except Exception as exc:
        result.update({"error": repr(exc), "traceback": traceback.format_exc()})
    return write_result(result, args.out)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="mode", required=True)
    meta = sub.add_parser("meta", help="ctl: decode local-file Meta into compact expected-copy manifest")
    meta.add_argument("--meta-store", required=True, type=Path, help="Meta data_dir/meta-store directory")
    meta.add_argument("--expected-sha256", default=EXPECTED_SHA256)
    meta.add_argument("--out", type=Path)
    meta.set_defaults(func=mode_meta)
    node = sub.add_parser("node", help="data node: verify original physical chunks in place")
    node.add_argument("--manifest", required=True, type=Path)
    node.add_argument("--node-id", required=True)
    node.add_argument("--chunk-root", required=True, type=Path, help="node data_dir/dfs root")
    node.add_argument("--out", type=Path)
    node.set_defaults(func=mode_node)
    combine = sub.add_parser("combine", help="ctl: combine one manifest and three node JSON reports")
    combine.add_argument("--manifest", required=True, type=Path)
    combine.add_argument("--node-report", action="append", required=True, help="node report JSON; repeat three times")
    combine.add_argument("--out", type=Path)
    combine.set_defaults(func=mode_combine)
    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
