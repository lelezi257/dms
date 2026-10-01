import importlib.util
import socket
import ssl
import struct
import threading
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("env_network", HERE / "env_network.py")
env_network = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(env_network)


class EnvNetworkUnitTest(unittest.TestCase):
    def _pair(self):
        left, right = socket.socketpair()
        self.addCleanup(left.close)
        self.addCleanup(right.close)
        return left, right

    def test_good_echo(self):
        left, right = self._pair()
        token = b"fresh-token"

        def echo():
            env_network.send_frame(right, env_network.recv_frame(right, 1.0))

        thread = threading.Thread(target=echo)
        thread.start()
        result = env_network.probe_stream(left, token, 1.0)
        thread.join(1.0)
        self.assertEqual("PASS", result["status"])
        self.assertEqual(len(token), result["bytes"])

    def test_mismatch_echo_fails(self):
        left, right = self._pair()

        def wrong():
            env_network.recv_frame(right, 1.0)
            env_network.send_frame(right, b"not-the-token")

        thread = threading.Thread(target=wrong)
        thread.start()
        result = env_network.probe_stream(left, b"expected-token", 1.0)
        thread.join(1.0)
        self.assertEqual({"status": "FAIL", "reason": "mismatch", "received_len": 13}, result)

    def test_timeout_fails(self):
        left, _ = self._pair()
        result = env_network.probe_stream(left, b"token", 0.05)
        self.assertEqual("FAIL", result["status"])
        self.assertEqual("timeout", result["reason"])

    def test_bounded_frame_rejects_oversize(self):
        left, right = self._pair()
        right.sendall(struct.pack("!I", env_network.MAX_FRAME + 1))
        with self.assertRaises(env_network.ProbeError) as raised:
            env_network.recv_frame(left, 1.0)
        self.assertEqual("frame_too_large", raised.exception.reason)

    def test_tls_negative_classification(self):
        cases = [
            (ssl.SSLCertVerificationError("certificate verify failed: unable to get local issuer certificate"), "untrusted_ca"),
            (ssl.CertificateError("hostname 'bad' doesn't match"), "wrong_hostname"),
            (ssl.SSLError("TLSV13_ALERT_CERTIFICATE_REQUIRED"), "missing_client_cert"),
            (ssl.SSLError("SSLV3_ALERT_BAD_CERTIFICATE"), "handshake_rejected"),
            (ConnectionResetError("reset"), "transport_reset_without_tls_alert"),
            (OSError("plain socket error"), "not_tls_handshake_rejection"),
        ]
        for error, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(expected, env_network.classify_tls_failure(error)["classification"])

    def test_check_selection_defaults_to_all_without_vacuous_empty(self):
        self.assertEqual({"tcp", "udp", "tls"}, env_network.selected_checks([]))
        self.assertEqual({"tcp", "udp", "tls"}, env_network.selected_checks(["all"]))
        self.assertEqual({"tcp"}, env_network.selected_checks(["tcp"]))

    def test_wrong_check_selection_is_rejected_by_parser(self):
        parser = env_network.build_parser()
        with self.assertRaises(SystemExit):
            parser.parse_args([
                "client",
                "--source-ip", "127.0.0.1",
                "--target-ip", "127.0.0.1",
                "--port", "19566",
                "--tls-port", "19567",
                "--ca", "ca.pem",
                "--client-cert", "client.pem",
                "--client-key", "client.key",
                "--server-hostname", "afs-env-a",
                "--check", "bogus",
            ])


if __name__ == "__main__":
    unittest.main()
