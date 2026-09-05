#!/usr/bin/env python3
"""将受限 Markdown 渲染为可审阅的确定性 HTML，仅使用 Python 标准库。

正文唯一来源是 Markdown。支持标题、段落、单层列表、表格、代码块、引用、
行内代码/加粗/链接；raw HTML 作为文字显示，不执行。sequence 代码块支持
participant A as 名称、A ->> B: 请求、B -->> A: 回答（包括自调用）。
不实现完整 CommonMark、嵌套列表、Mermaid 或项目特定架构图。
baseline 只能是调用者明确声明“人工已确认”的独立 Markdown 快照；工具不能
代替人的确认。禁止使用输出 HTML 自动提升 baseline，以免掩盖未经确认的变更。
"""

import argparse
import base64
import difflib
import hashlib
import html
import os
from pathlib import Path
import re
from urllib.parse import quote, unquote, urlsplit, urlunsplit

VERSION = "1.0.0"


def digest(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def safe_link(url, source, output):
    """只允许 HTTP(S)、mailto 和文档路径；路径相对 Markdown 所在目录解析。"""
    url = url.strip().removeprefix("<").removesuffix(">")
    if any(ord(char) < 32 for char in url) or "\\" in url:
        raise ValueError("链接包含控制字符或反斜线")
    parsed = urlsplit(url)
    if parsed.scheme:
        if parsed.scheme.lower() not in {"http", "https", "mailto"}:
            raise ValueError(f"不允许的链接协议: {parsed.scheme}")
        return url
    if parsed.netloc or url.startswith("//"):
        raise ValueError("不允许协议相对链接")
    if not parsed.path:
        return url  # 同一 HTML 中的 #heading 保持不变。
    path = Path(unquote(parsed.path))
    target = path if path.is_absolute() else source.parent / path
    if target.resolve() == source.resolve():
        target = output  # 本文.md#heading 指向对应 HTML 自身。
    relative = os.path.relpath(target.resolve(), output.parent.resolve())
    return urlunsplit(("", "", quote(relative, safe="/.-_~"), parsed.query, parsed.fragment))


def inline(text, source, output):
    """先识别允许的行内结构，再转义其它文字；任何输入不能注入 HTML。"""
    tokens = re.compile(r"`[^`]+`|\*\*[^*]+\*\*|\[[^\]]+\]\((?:<[^>]+>|[^)]+)\)")
    fragments, start = [], 0
    for match in tokens.finditer(text):
        fragments.append(html.escape(text[start:match.start()]))
        token = match.group()
        if token.startswith("`"):
            fragments.append(f"<code>{html.escape(token[1:-1])}</code>")
        elif token.startswith("**"):
            fragments.append(f"<strong>{html.escape(token[2:-2])}</strong>")
        else:
            label, url = re.fullmatch(r"\[([^\]]+)\]\((.+)\)", token).groups()
            fragments.append(f'<a href="{html.escape(safe_link(url, source, output), quote=True)}">'
                             f'{html.escape(label)}</a>')
        start = match.end()
    fragments.append(html.escape(text[start:]))
    return "".join(fragments)


def differences(current, baseline):
    changes, previous = {}, []
    if baseline is None:
        return changes, previous
    old, new = baseline.splitlines(), current.splitlines()
    for tag, a, b, c, d in difflib.SequenceMatcher(a=old, b=new, autojunk=False).get_opcodes():
        if tag == "equal":
            continue
        for line in range(c + 1, d + 1):
            # SequenceMatcher 会把“改末行 + 追加段落”合成 replace；超过旧行数
            # 的尾部仍应标记新增。这里只解释行差异，不推断业务变更含义。
            changes[line] = "added" if tag == "insert" or line - c > b - a else "modified"
        if tag in {"delete", "replace"}:
            previous.append((a + 1, b, "删除" if tag == "delete" else "修改前", "\n".join(old[a:b])))
    return changes, previous


def sequence(lines):
    """通用时序图：SVG 节点/边/标签均由 DSL 生成，原文同时保留可展开。"""
    participants, messages = {}, []
    for line in lines:
        if not line.strip():
            continue
        actor = re.fullmatch(r"participant\s+(\w+)\s+as\s+(.+)", line.strip())
        message = re.fullmatch(r"(\w+)\s*(-->>|->>)\s*(\w+)\s*:\s*(.+)", line.strip())
        if actor:
            alias, label = actor.groups()
            if alias in participants:
                raise ValueError(f"重复 participant: {alias}")
            participants[alias] = label
        elif message:
            messages.append(message.groups())
        else:
            raise ValueError(f"不支持的 sequence 语句: {line}")
    if not participants:
        raise ValueError("sequence 缺少 participant")
    if any(a not in participants or b not in participants for a, _, b, _ in messages):
        raise ValueError("sequence 引用了未声明的 participant")
    width, height = max(600, len(participants) * 260), 120 + len(messages) * 66
    positions = {name: 130 + index * 260 for index, name in enumerate(participants)}
    result = [f'<div class="diagram"><svg xmlns="http://www.w3.org/2000/svg" '
              f'width="{width}" height="{height}" viewBox="0 0 {width} {height}" '
              'role="img" aria-label="由 Markdown sequence 生成的时序图">']
    for name, label in participants.items():
        x = positions[name]
        result += [f'<rect x="{x-110}" y="12" width="220" height="44" rx="8" fill="#e5f0ff" stroke="#516d92"/>',
                   f'<text x="{x}" y="39" text-anchor="middle">{html.escape(label)}</text>',
                   f'<path d="M{x} 56 V{height-15}" stroke="#999" stroke-dasharray="5 5"/>']
    for index, (a, arrow, b, label) in enumerate(messages):
        x1, x2, y = positions[a], positions[b], 96 + index * 66
        dashed = ' stroke-dasharray="6 4"' if arrow == "-->>" else ""
        path = f"M{x1} {y} H{x2}" if a != b else f"M{x1} {y} h75 v22 h-75"
        end_y = y if a != b else y + 22
        side = -1 if x2 > x1 else 1
        result += [f'<path d="{path}" stroke="#344054" fill="none"{dashed}/>',
                   f'<path d="M{x2+side*9} {end_y-5} L{x2} {end_y} L{x2+side*9} {end_y+5}" '
                   'stroke="#344054" fill="none"/>',
                   f'<text x="{(x1+x2)/2}" y="{y-9}" text-anchor="middle">{html.escape(label)}</text>']
    result += ['</svg></div>', '<details><summary>查看 Markdown 原图</summary><pre><code>',
               html.escape("\n".join(lines)), '</code></pre></details>']
    return "".join(result)


def cells(line):
    return [part.strip().replace(r"\|", "|") for part in re.split(r"(?<!\\)\|", line.strip().strip("|"))]


def table_separator(line):
    parts = cells(line)
    return bool(parts) and all(re.fullmatch(r":?-{3,}:?", part) for part in parts)


def render_body(text, source, output, changes):
    lines, result, toc, anchors = text.splitlines(), [], [], {}

    def kind(start, end):
        found = {changes[number] for number in range(start, end + 1) if number in changes}
        return "unchanged" if not found else "added" if found == {"added"} else "modified"

    def unit(tag, content, start, end, extra=""):
        change = kind(start, end)
        badge = "" if change == "unchanged" else (
            '<span class="badge">本轮新增</span>' if change == "added" else '<span class="badge">本轮修改</span>')
        return f'<{tag} class="unit {change}" data-lines="{start}-{end}"{extra}>{badge}{content}</{tag}>'

    def fmt(value):
        return inline(value, source, output)

    def starts_block(index):
        candidate = lines[index].strip()
        return (not candidate or re.match(r"^(#{1,6}\s|```|>|[-*]\s|\d+\.\s)", candidate)
                or index + 1 < len(lines) and "|" in candidate and table_separator(lines[index + 1]))

    index = 0
    while index < len(lines):
        value, start = lines[index].strip(), index + 1
        if re.match(r"^(?: {2,}|\t)[-*\d].*", lines[index]) and re.match(r"(?:[-*]|\d+\.)\s", value):
            raise ValueError(f"第 {start} 行为嵌套/缩进列表；请改为单层列表或原文代码块")
        if not value:
            index += 1
            continue
        if value.startswith("```"):
            language, code = value[3:].strip(), []
            index += 1
            while index < len(lines) and lines[index].strip() != "```":
                code.append(lines[index])
                index += 1
            if index == len(lines):
                raise ValueError(f"第 {start} 行代码块未闭合")
            index += 1
            content = sequence(code) if language == "sequence" else (
                f'<pre><code class="language-{html.escape(language, quote=True)}">'
                f'{html.escape(chr(10).join(code))}</code></pre>')
            result.append(unit("section", content, start, index))
            continue
        heading = re.fullmatch(r"(#{1,6})\s+(.+)", value)
        if heading:
            level, title = len(heading[1]), heading[2]
            slug = re.sub(r"[^\w\s-]", "", title.lower()).strip().replace(" ", "-") or "section"
            count = anchors.get(slug, 0)
            anchors[slug] = count + 1
            anchor = slug if count == 0 else f"{slug}-{count}"
            toc.append(f'<li><a href="#{html.escape(anchor, quote=True)}">{html.escape(title)}</a></li>')
            result.append(unit(f"h{level}", fmt(title), start, start, f' id="{html.escape(anchor, quote=True)}"'))
            index += 1
            continue
        if index + 1 < len(lines) and "|" in value and table_separator(lines[index + 1]):
            headers = cells(value)
            head_kind = kind(start, start + 1)
            rows = []
            index += 2
            while index < len(lines) and lines[index].strip() and "|" in lines[index]:
                row = cells(lines[index])
                if len(row) != len(headers):
                    raise ValueError(f"第 {index+1} 行表格列数不一致")
                rendered = "".join(f"<td>{fmt(cell)}</td>" for cell in row)
                # 标识必须放在单元格内；tr 的直接子元素只能是 th/td。
                rendered = unit("tr", rendered, index + 1, index + 1)
                rendered = re.sub(r'(<tr[^>]*>)(<span class="badge">.*?</span>)(<td>)', r'\1\3\2', rendered)
                rows.append(rendered)
                index += 1
            head_badge = "" if head_kind == "unchanged" else '<span class="badge">本轮表头变更</span>'
            head = "".join(f"<th>{head_badge if pos == 0 else ''}{fmt(cell)}</th>" for pos, cell in enumerate(headers))
            changed = head_kind != "unchanged" or any('class="unit unchanged"' not in row for row in rows)
            result.append(f'<div class="compound {"has-changes" if changed else "unchanged"}"><table>'
                          f'<thead><tr>{head}</tr></thead><tbody>{"".join(rows)}</tbody></table></div>')
            continue
        item = re.fullmatch(r"([-*]|\d+\.)\s+(.+)", value)
        if item:
            ordered = item[1][0].isdigit()
            pattern = r"\d+\.\s+(.+)" if ordered else r"[-*]\s+(.+)"
            items = []
            while index < len(lines):
                if lines[index].startswith(("  ", "\t")):
                    break
                match = re.fullmatch(pattern, lines[index].strip())
                if not match:
                    break
                items.append(unit("li", fmt(match[1]), index + 1, index + 1))
                index += 1
            changed = any('class="unit unchanged"' not in item for item in items)
            tag = "ol" if ordered else "ul"
            start_attr = f' start="{int(item[1][:-1])}"' if ordered else ""
            result.append(f'<{tag} class="compound {"has-changes" if changed else "unchanged"}"{start_attr}>{"".join(items)}</{tag}>')
            continue
        paragraph = [value[1:].strip() if value.startswith(">") else value]
        tag = "blockquote" if value.startswith(">") else "p"
        index += 1
        while index < len(lines) and not starts_block(index):
            paragraph.append(lines[index].strip())
            index += 1
        result.append(unit(tag, fmt(" ".join(paragraph)), start, index))
    return "\n".join(result), "".join(toc)


STYLE = """
:root{font:16px/1.65 system-ui,sans-serif;color:#172b43;background:#f3f6fa}
body{margin:0}main{max-width:1080px;margin:auto;background:white;padding:32px}
header,nav,footer{max-width:1080px;margin:16px auto;padding:20px;background:white;border-radius:10px}
h1,h2,h3{line-height:1.3;scroll-margin-top:20px}h2{margin-top:2em}a{color:#175cd3}
pre{overflow:auto;background:#102237;color:#eff5fc;padding:18px;border-radius:8px}code{font-family:monospace}
table{border-collapse:collapse;width:100%}td,th{border:1px solid #cdd5df;padding:9px;text-align:left}
.compound,.diagram{overflow-x:auto}.diagram text{font:14px system-ui,sans-serif}
.unit{border-left:3px solid transparent;padding-left:10px}.added{background:#e8f8ee;border-color:#16824b}
.modified{background:#eaf2ff;border-color:#236ed2}.badge{display:inline-block;font-size:12px;font-weight:bold;margin-right:10px}
body.diff-only main .unit.unchanged,body.diff-only main .compound.unchanged{display:none}
details{margin:12px 0}summary,button{cursor:pointer}button{padding:8px 16px}blockquote{color:#475467}
.metadata{font-size:12px;overflow-wrap:anywhere}.previous{border:1px solid #b4bcc7;padding:10px}
@media(max-width:700px){main,header,nav,footer{padding:15px;margin:8px}table{min-width:520px}}
"""


def render(source_text, source, output, baseline_text=None, baseline=None, round_label=""):
    changes, previous = differences(source_text, baseline_text)
    body, toc = render_body(source_text, source, output, changes)
    source_sha = digest(source_text)
    baseline_sha = digest(baseline_text) if baseline_text is not None else ""
    metadata = (f'源 Markdown：{html.escape(source.as_posix())}<br>SHA-256：{source_sha}<br>'
                f'Renderer：{VERSION}<br>Baseline：{html.escape(baseline.as_posix()) if baseline else "未设置"}'
                f'<br>Baseline SHA-256：{baseline_sha or "未设置"}')
    review = "全量阅读视图；HTML 不得手工修改。"
    if baseline_text is not None:
        review = (f'Review 元数据 · {html.escape(round_label)} · 新增 {list(changes.values()).count("added")} 行 · '
                  f'修改 {list(changes.values()).count("modified")} 行 · 删除/替换前 {sum(b-a+1 for a,b,_,_ in previous)} 行 '
                  '<button id="toggle" type="button" aria-pressed="false">只看本轮变化</button>')
    history = "".join(f'<details class="previous"><summary>{label} · baseline {a}–{b} 行</summary>'
                      f'<pre>{html.escape(text)}</pre></details>' for a, b, label, text in previous)
    encoded = base64.b64encode(source_text.encode()).decode()
    old_encoded = base64.b64encode(baseline_text.encode()).decode() if baseline_text is not None else ""
    return f'''<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="source-path" content="{html.escape(source.as_posix(), quote=True)}">
<meta name="source-sha256" content="{source_sha}"><meta name="baseline-sha256" content="{baseline_sha}">
<meta name="renderer-version" content="{VERSION}"><title>{html.escape(source.stem)} · 阅读视图</title>
<style>{STYLE}</style></head><body>
<header><strong>自动生成的 Markdown 阅读视图</strong><p>{review}</p><div class="metadata">{metadata}</div></header>
<nav aria-label="目录"><details><summary>目录</summary><ul>{toc}</ul></details></nav>
<main>{body}</main><footer><strong>Review 元数据：旧内容</strong>{history or "无删除或替换前内容"}</footer>
<script id="embedded-markdown" type="application/octet-stream">{encoded}</script>
<script id="embedded-baseline" type="application/octet-stream">{old_encoded}</script>
<script>const toggle=document.getElementById('toggle');if(toggle)toggle.addEventListener('click',()=>{{
const active=document.body.classList.toggle('diff-only');toggle.setAttribute('aria-pressed',String(active));
toggle.textContent=active?'显示完整文档':'只看本轮变化';}});</script></body></html>
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--baseline-confirmed", action="store_true", help="声明此独立 Markdown 快照已经人工确认")
    parser.add_argument("--round-label", default="")
    parser.add_argument("--check", action="store_true", help="只校验，不修改任何输出")
    args = parser.parse_args()
    try:
        if args.source.suffix.lower() != ".md" or args.output.suffix.lower() != ".html":
            raise ValueError("输入必须是 .md，输出必须是 .html")
        if args.source.resolve() == args.output.resolve():
            raise ValueError("输出不能覆盖源文件")
        if bool(args.baseline) != args.baseline_confirmed:
            raise ValueError("--baseline 与 --baseline-confirmed 必须同时提供")
        baseline_text = None
        if args.baseline:
            if args.baseline.suffix.lower() != ".md" or args.baseline.resolve() in {args.source.resolve(), args.output.resolve()}:
                raise ValueError("baseline 必须是独立的已确认 .md 快照")
            baseline_text = args.baseline.read_bytes().decode("utf-8")
        text = args.source.read_bytes().decode("utf-8")
        expected = render(text, args.source, args.output, baseline_text, args.baseline, args.round_label)
        if args.check:
            if args.output.read_bytes() != expected.encode("utf-8"):
                raise ValueError("Markdown/HTML 漂移：请重新生成，不能手工修改 HTML")
        else:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_bytes(expected.encode("utf-8"))
        print(f'{"OK" if args.check else "WROTE"} {args.output} sha256={digest(text)} renderer={VERSION}')
    except (OSError, ValueError, UnicodeError) as error:
        parser.exit(1, f"ERROR: {error}\n")


if __name__ == "__main__":
    main()
