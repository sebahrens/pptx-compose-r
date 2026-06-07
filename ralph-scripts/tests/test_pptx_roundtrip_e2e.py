import importlib.util
import pathlib
import sys
import tempfile
import unittest
from unittest import mock


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "ralph-scripts" / "pptx_roundtrip_e2e.py"
LOOP = REPO_ROOT / "ralph-scripts" / "loop.sh"


def load_runner():
    spec = importlib.util.spec_from_file_location("pptx_roundtrip_e2e", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class RoundTripE2ETest(unittest.TestCase):
    def test_loop_exposes_and_runs_roundtrip_e2e_under_test_flag(self):
        loop = LOOP.read_text()

        self.assertIn("roundtrip-e2e", loop)
        self.assertIn("RUN_ROUNDTRIP_E2E", loop)
        self.assertIn("pptx_roundtrip_e2e.py", loop)
        self.assertIn("include-tests", loop)

    def test_opinion_prefers_failures_over_visual_skips(self):
        runner = load_runner()

        report = runner.FixtureReport(
            fixture="fixtures/legacy/sample.pptx",
            status="pass",
            commands=[],
            xml=runner.ComparisonResult(status="pass"),
            media=runner.ComparisonResult(status="pass"),
            visual=runner.ComparisonResult(status="skipped", details=["LibreOffice not installed"]),
            validation=runner.ComparisonResult(status="pass"),
            output_pptx="out.pptx",
            json_path="out.json",
            log_path="roundtrip.log",
        )
        self.assertEqual(runner.form_opinion([report]).status, "pass")

        report.xml = runner.ComparisonResult(status="fail", details=["slide1.xml differs"])
        opinion = runner.form_opinion([report])

        self.assertEqual(opinion.status, "fail")
        self.assertIn("XML/package comparison failed", "\n".join(opinion.reasons))

    def test_issue_description_contains_comparison_and_log_context(self):
        runner = load_runner()
        report = runner.FixtureReport(
            fixture="fixtures/minimal.pptx",
            status="fail",
            commands=["pptx-compose to-json ...", "pptx-compose to-pptx ..."],
            xml=runner.ComparisonResult(status="fail", details=["ppt/slides/slide1.xml differs"]),
            media=runner.ComparisonResult(status="pass"),
            visual=runner.ComparisonResult(status="skipped", details=["pdftoppm not installed"]),
            validation=runner.ComparisonResult(status="pass"),
            output_pptx="/tmp/out.pptx",
            json_path="/tmp/out.json",
            log_path="/tmp/roundtrip.log",
        )

        description = runner.issue_description(report)

        self.assertIn("fixtures/minimal.pptx", description)
        self.assertIn("ppt/slides/slide1.xml differs", description)
        self.assertIn("/tmp/roundtrip.log", description)
        self.assertIn("Visual comparison", description)

    def test_visual_input_pdf_render_failure_is_skipped(self):
        runner = load_runner()

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(runner.shutil, "which") as which:
            tmp_path = pathlib.Path(tmp)
            which.side_effect = lambda name: f"/usr/bin/{name}"
            with mock.patch.object(runner, "render_pdf", return_value=None) as render_pdf:
                result = runner.compare_visual(
                    tmp_path / "input.pptx",
                    tmp_path / "output.pptx",
                    tmp_path / "visual",
                    0.5,
                    tmp_path / "roundtrip.log",
                )

        self.assertEqual(result.status, "skipped")
        self.assertIn("input visual PDF render failed", "\n".join(result.details))
        self.assertEqual(render_pdf.call_count, 1)

    def test_visual_output_pdf_render_failure_is_fail_after_input_renders(self):
        runner = load_runner()

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(runner.shutil, "which") as which:
            tmp_path = pathlib.Path(tmp)
            input_pdf = tmp_path / "input.pdf"
            which.side_effect = lambda name: f"/usr/bin/{name}"
            with mock.patch.object(runner, "render_pdf", side_effect=[input_pdf, None]):
                result = runner.compare_visual(
                    tmp_path / "input.pptx",
                    tmp_path / "output.pptx",
                    tmp_path / "visual",
                    0.5,
                    tmp_path / "roundtrip.log",
                )

        self.assertEqual(result.status, "fail")
        self.assertIn("output visual PDF render failed", "\n".join(result.details))

    def test_visual_input_image_render_failure_is_skipped(self):
        runner = load_runner()

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(runner.shutil, "which") as which:
            tmp_path = pathlib.Path(tmp)
            which.side_effect = lambda name: f"/usr/bin/{name}"
            with (
                mock.patch.object(runner, "render_pdf", side_effect=[tmp_path / "input.pdf", tmp_path / "output.pdf"]),
                mock.patch.object(runner, "render_images", return_value=[]),
            ):
                result = runner.compare_visual(
                    tmp_path / "input.pptx",
                    tmp_path / "output.pptx",
                    tmp_path / "visual",
                    0.5,
                    tmp_path / "roundtrip.log",
                )

        self.assertEqual(result.status, "skipped")
        self.assertIn("input visual image render failed", "\n".join(result.details))

    def test_visual_output_image_render_failure_is_fail_after_input_renders(self):
        runner = load_runner()

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(runner.shutil, "which") as which:
            tmp_path = pathlib.Path(tmp)
            which.side_effect = lambda name: f"/usr/bin/{name}"
            with (
                mock.patch.object(runner, "render_pdf", side_effect=[tmp_path / "input.pdf", tmp_path / "output.pdf"]),
                mock.patch.object(runner, "render_images", side_effect=[[tmp_path / "input-1.png"], []]),
            ):
                result = runner.compare_visual(
                    tmp_path / "input.pptx",
                    tmp_path / "output.pptx",
                    tmp_path / "visual",
                    0.5,
                    tmp_path / "roundtrip.log",
                )

        self.assertEqual(result.status, "fail")
        self.assertIn("output visual image render failed", "\n".join(result.details))


if __name__ == "__main__":
    unittest.main()
