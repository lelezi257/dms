"""标准库测试：在 Linux VM 内运行 python3 -m unittest discover -s scripts/docs。"""

import base64
from html.parser import HTMLParser
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

import render_review as renderer


class Elements(HTMLParser):
    def __init__(self):
        super().__init__()
        self.elements = []

    def handle_starttag(self, tag, attrs):
        self.elements.append((tag, dict(attrs)))


class ReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "review.md"
        self.output = self.root / "review.html"
        self.source.write_text("# 验收\n\n正文。\n", encoding="utf-8")

    def cli(self, *extra):
        return subprocess.run([sys.executable, str(Path(renderer.__file__).resolve()),
                               str(self.source), str(self.output), *map(str, extra)],
                              capture_output=True, text=True, check=False)

    def test_embedded_source_is_byte_exact_and_deterministic(self):
        text = "# 中文\r\n\r\n值 `abc` <script>alert(1)</script>。\r\n"
        document = renderer.render(text, self.source, self.output)
        self.assertEqual(document, renderer.render(text, self.source, self.output))
        encoded = re.search(r'id="embedded-markdown" type="application/octet-stream">([^<]*)', document)[1]
        self.assertEqual(base64.b64decode(encoded).decode(), text)
        self.assertIn(f'content="{renderer.digest(text)}"', document)
        self.assertIn('name="source-path"', document)
        self.assertIn('name="renderer-version"', document)
        self.assertNotIn("<script>alert", document)
        self.assertIn("&lt;script&gt;alert(1)&lt;/script&gt;", document)

    def test_supported_syntax_and_source_driven_sequence(self):
        text = ("# 阅读\n\n**重点** [文档](guide.md)\n\n- 列表项\n\n3. 第三项\n\n"
                "| 列 | 值 |\n| --- | --- |\n| A | `B` |\n\n> 引用\n\n"
                "```rust\nlet x = 1;\n```\n\n"
                "```sequence\nparticipant C as 调用方\nparticipant N as 服务方\n"
                "C ->> N: 保存\nN ->> N: 内部处理\nN -->> C: 已保存\n```\n")
        document = renderer.render(text, self.source, self.output)
        for fragment in ('<strong>重点</strong>', '<ol class="compound unchanged" start="3">',
                         '<th>列</th>', '<td><code>B</code></td>', '<blockquote', 'language-rust', '<svg',
                         '>调用方</text>', '>保存</text>', '>已保存</text>', '查看 Markdown 原图'):
            self.assertIn(fragment, document)
        self.assertIn('d="M390 162 h75 v22 h-75"', document)

    def test_diff_rows_and_items_keep_individual_visibility(self):
        old = "# 审阅\n\n- 不变项\n- 旧项\n\n| 字段 | 含义 |\n| --- | --- |\n| A | 不变 |\n| B | 旧值 |\n"
        new = old.replace("旧项", "新项").replace("旧值", "新值") + "\n新增段落。\n"
        document = renderer.render(new, self.source, self.output, old, self.root / "accepted.md", "R2")
        self.assertIn('<li class="unit unchanged"', document)
        self.assertIn('<li class="unit modified"', document)
        self.assertIn('<tr class="unit unchanged"', document)
        self.assertIn('<tr class="unit modified"', document)
        self.assertIn('body.diff-only main .unit.unchanged', document)
        self.assertIn('body.diff-only main .compound.unchanged{display:none}', document)
        self.assertIn('classList.toggle(\'diff-only\')', document)
        self.assertIn('aria-pressed="false"', document)
        self.assertIn('本轮新增', document)
        self.assertIn('本轮修改', document)
        self.assertIn('新增 2 行 · 修改 2 行 · 删除/替换前 2 行', document)
        self.assertIn('<details class="previous"><summary>修改前', document)
        self.assertNotIn('<details class="previous" open', document)
        # 表格变更标识必须处在 td 内，避免浏览器把 span 移到整张表之外。
        self.assertRegex(document, r'<tr class="unit modified"[^>]*><td><span class="badge">')
        encoded = re.search(r'id="embedded-baseline" type="application/octet-stream">([^<]*)', document)[1]
        self.assertEqual(base64.b64decode(encoded).decode(), old)
        self.assertIn(f'content="{renderer.digest(old)}"', document)

    def test_header_only_diff_remains_visible(self):
        old = "| 名称 |\n| --- |\n| A |\n"
        doc = renderer.render(old.replace("名称", "新名称"), self.source, self.output, old, self.root / "accepted.md")
        self.assertIn('class="compound has-changes"', doc)
        self.assertIn('本轮表头变更', doc)

    def test_deleted_content_is_folded_without_current_invention(self):
        old = "# 当前\n\n消失的结论\n"
        doc = renderer.render("# 当前\n", self.source, self.output, old, self.root / "accepted.md")
        current = doc.split("<main>", 1)[1].split("</main>", 1)[0]
        self.assertNotIn("消失的结论", current)
        self.assertIn("<summary>删除", doc)

    def test_links_preserve_anchors_and_rebase_relative_files(self):
        text = "# 中文 标题\n\n[跳转](#中文-标题) [本文](review.md#中文-标题) [其它](guide.md#usage)"
        output = self.root / "views" / "review.html"
        doc = renderer.render(text, self.source, output)
        self.assertIn('id="中文-标题"', doc)
        self.assertIn('href="#中文-标题"', doc)
        self.assertIn('href="review.html#中文-标题"', doc)
        self.assertIn('href="../guide.md#usage"', doc)

    def test_duplicate_heading_ids(self):
        doc = renderer.render("# 标题\n\n## 标题\n", self.source, self.output)
        self.assertIn('id="标题"', doc)
        self.assertIn('id="标题-1"', doc)

    def test_report_links_across_outer_directories(self):
        source = self.root / "design" / "report.md"
        output = self.root / "outputs" / "stages" / "report.html"
        text = "# 总结\n\n[源码](../source/README.md) [证据](../evidence/result.json) [本文](report.md#总结)"
        doc = renderer.render(text, source, output)
        self.assertIn('href="../../source/README.md"', doc)
        self.assertIn('href="../../evidence/result.json"', doc)
        self.assertIn('href="report.html#总结"', doc)

    def test_unsafe_links_rejected_not_executed(self):
        for url in ("javascript:alert", "data:text/html,hello", "vbscript:evil", "//evil.test/x", "bad\\path", "https://a\t.test"):
            with self.subTest(url=url), self.assertRaises(ValueError):
                renderer.render(f"[链接]({url})", self.source, self.output)
        for url in ("https://example.org/x", "http://example.org", "mailto:hello@example.org"):
            self.assertIn(f'href="{url}"', renderer.render(f"[安全]({url})", self.source, self.output))

    def test_raw_html_escaped(self):
        doc = renderer.render('<iframe src="evil"></iframe>\n<img src=x onerror=evil>', self.source, self.output)
        parser = Elements()
        parser.feed(doc)
        self.assertNotIn("iframe", [tag for tag, _ in parser.elements])
        self.assertNotIn("img", [tag for tag, _ in parser.elements])

    def test_invalid_sequence_and_tables_fail(self):
        for text in ("```sequence\nparticipant C as Client\nC ->> N: x\n```",
                     "```sequence\nparticipant C as Client\nparticipant C as Again\n```",
                     "```rust\nunclosed", "| A | B |\n| --- | --- |\n| x |",
                     "- 外层\n  - 内层"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                renderer.render(text, self.source, self.output)

    def test_cli_check_detects_source_and_html_drift(self):
        self.assertEqual(self.cli().returncode, 0)
        original = self.output.read_bytes()
        self.assertEqual(self.cli("--check").returncode, 0)
        self.output.write_bytes(original + b"manual edit")
        self.assertNotEqual(self.cli("--check").returncode, 0)
        self.assertTrue(self.output.read_bytes().endswith(b"manual edit"))
        self.output.write_bytes(original)
        self.source.write_text("# 修改后\n", encoding="utf-8")
        self.assertNotEqual(self.cli("--check").returncode, 0)

    def test_baseline_requires_explicit_independent_markdown_and_check(self):
        baseline = self.root / "accepted.md"
        baseline.write_text("# 旧稿\n", encoding="utf-8")
        for args in (("--baseline", baseline), ("--baseline-confirmed",),
                     ("--baseline", self.source, "--baseline-confirmed"),
                     ("--baseline", self.output, "--baseline-confirmed")):
            with self.subTest(args=args):
                self.assertNotEqual(self.cli(*args).returncode, 0)
        args = ("--baseline", baseline, "--baseline-confirmed", "--round-label", "R2")
        self.assertEqual(self.cli(*args).returncode, 0)
        self.assertEqual(self.cli(*args, "--check").returncode, 0)
        baseline.write_text("# 快照被误改\n", encoding="utf-8")
        self.assertNotEqual(self.cli(*args, "--check").returncode, 0)

    def test_standalone_copy_has_no_repository_dependency(self):
        script = self.root / "render_review.py"
        shutil.copyfile(renderer.__file__, script)
        command = [sys.executable, "-I", str(script), str(self.source), str(self.output)]
        generated = subprocess.run(command, cwd=self.root, capture_output=True, text=True, check=False)
        self.assertEqual(generated.returncode, 0, generated.stderr)
        checked = subprocess.run(command + ["--check"], cwd=self.root, capture_output=True, text=True, check=False)
        self.assertEqual(checked.returncode, 0, checked.stderr)


if __name__ == "__main__":
    unittest.main()
