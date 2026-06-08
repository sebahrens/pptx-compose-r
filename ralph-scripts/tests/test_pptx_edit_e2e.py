"""Tests for the PPTX EDIT round-trip E2E harness (pptx_edit_e2e.py).

Two layers:

* Pure-logic unit tests (always run, no Rust binary needed): patch building,
  target discovery from an agent view, structure assertions (targeted parts
  changed vs unrelated parts byte-identical), validation gating, and the
  graceful-degradation opinion logic.

* Integration tests (skipped unless the ``pptx-compose`` debug binary exists):
  actually inspect a real fixture, apply a representative patch per V1 op, and
  assert the targeted text/structure changed while unrelated parts stay
  byte-identical and post-edit validation is clean. The visual stage runs when
  LibreOffice/pdftoppm/Pillow are present and is treated as inconclusive
  (never a failure) otherwise.
"""

from __future__ import annotations

import importlib.util
import io
import pathlib
import subprocess
import sys
import unittest
import zipfile
from unittest import mock


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
EDIT_SCRIPT = REPO_ROOT / "ralph-scripts" / "pptx_edit_e2e.py"
LOOP = REPO_ROOT / "ralph-scripts" / "loop.sh"
CLI_BIN = REPO_ROOT / "target" / "debug" / "pptx-compose"


def load_edit_module():
    spec = importlib.util.spec_from_file_location("pptx_edit_e2e", EDIT_SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def make_zip(entries: dict[str, bytes]) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)
    return buf.getvalue()


def sample_view() -> dict:
    return {
        "schema": "pptx-compose.agent-view.v1",
        "version": 1,
        "document_id": "sha256:deadbeef",
        "revision": 4,
        "slides": [
            {
                "id": "slide-1",
                "part": "ppt/slides/slide1.xml",
                "elements": [
                    {
                        "id": "slide-1:shape-2",
                        "kind": "shape",
                        "part": "ppt/slides/slide1.xml",
                        "slide_id": "slide-1",
                        "fingerprint": "sha256:fp",
                        "bounds": {"x": 100, "y": 200, "cx": 300, "cy": 400},
                        "editable": {
                            "text": {"supported": True},
                            "image": {"supported": False, "reason": "not_picture"},
                            "bounds": {"supported": True},
                        },
                        "text": {
                            "plain": "Hello world\r\n",
                            "text_hash": "sha256:th",
                        },
                    },
                    {
                        "id": "slide-1:pic-3",
                        "kind": "image",
                        "part": "ppt/slides/slide1.xml",
                        "slide_id": "slide-1",
                        "bounds": {"x": 1, "y": 2, "cx": 3, "cy": 4},
                        "editable": {
                            "text": {"supported": False, "reason": "not_text"},
                            "image": {"supported": True},
                            "bounds": {"supported": True},
                        },
                    },
                ],
            }
        ],
    }


class PatchBuildingTest(unittest.TestCase):
    def setUp(self):
        self.m = load_edit_module()

    def test_build_patch_carries_document_and_revision_guards(self):
        patch = self.m.build_patch(sample_view(), [{"op": "noop"}])
        self.assertEqual(patch["schema"], "pptx-compose.patch.v1")
        self.assertEqual(patch["document_id"], "sha256:deadbeef")
        self.assertEqual(patch["base_revision"], 4)
        self.assertEqual(patch["operations"], [{"op": "noop"}])

    def test_editable_text_elements_filters_on_supported_and_text(self):
        els = self.m.editable_text_elements(sample_view())
        self.assertEqual([e["id"] for e in els], ["slide-1:shape-2"])

    def test_picture_elements_requires_image_support(self):
        els = self.m.picture_elements(sample_view())
        self.assertEqual([e["id"] for e in els], ["slide-1:pic-3"])

    def test_replace_text_builder_uses_text_hash_guard_and_marker(self):
        plan = self.m.build_replace_text(sample_view(), {})
        self.assertEqual(len(plan.operations), 1)
        op = plan.operations[0]
        self.assertEqual(op["op"], "replace_text")
        self.assertIn(self.m.TEXT_MARKER, op["text"])
        self.assertEqual(op["selector"]["guards"]["text_hash"], "sha256:th")
        self.assertNotIn("element_id", op)
        self.assertIn("ppt/slides/slide1.xml", plan.expected_changed_parts)
        self.assertEqual(plan.expected_markers[self.m.TEXT_MARKER], "ppt/slides/slide1.xml")

    def test_set_alt_text_builder(self):
        plan = self.m.build_set_alt_text(sample_view(), {})
        op = plan.operations[0]
        self.assertEqual(op["op"], "set_alt_text")
        self.assertEqual(op["alt_text"], self.m.ALT_TEXT_MARKER)

    def test_add_image_builder_requires_media_source(self):
        plan = self.m.build_add_image(sample_view(), {})
        self.assertIsNotNone(plan.skip_reason)

        plan2 = self.m.build_add_image(sample_view(), {"png_source": "/tmp/x.png"})
        op = plan2.operations[0]
        self.assertEqual(op["op"], "add_image")
        self.assertEqual(plan2.media, {"e2e_added": "/tmp/x.png"})
        self.assertIn("ppt/slides/slide1.xml", plan2.expected_changed_parts)
        self.assertIn("ppt/slides/_rels/slide1.xml.rels", plan2.expected_changed_parts)

    def test_replace_image_builder_targets_rels(self):
        plan = self.m.build_replace_image(sample_view(), {"png_replacement": "/tmp/r.png"})
        op = plan.operations[0]
        self.assertEqual(op["op"], "replace_image")
        self.assertEqual(op["element_id"], "slide-1:pic-3")
        self.assertEqual(plan.expected_changed_parts, {"ppt/slides/_rels/slide1.xml.rels"})

    def test_rels_part_for(self):
        self.assertEqual(
            self.m._rels_part_for("ppt/slides/slide7.xml"),
            "ppt/slides/_rels/slide7.xml.rels",
        )


class StructureAssertionTest(unittest.TestCase):
    def setUp(self):
        self.m = load_edit_module()

    def _write(self, tmp, name, entries):
        p = tmp / name
        p.write_bytes(make_zip(entries))
        return p

    def test_targeted_change_with_unrelated_identical_passes(self):
        import tempfile

        with tempfile.TemporaryDirectory() as t:
            tmp = pathlib.Path(t)
            inp = self._write(tmp, "in.pptx", {
                "ppt/slides/slide1.xml": b"<old/>",
                "ppt/slides/slide2.xml": b"<keep/>",
            })
            out = self._write(tmp, "out.pptx", {
                "ppt/slides/slide1.xml": b"<new>PPTX_COMPOSE_E2E_EDIT_MARKER</new>",
                "ppt/slides/slide2.xml": b"<keep/>",
            })
            plan = self.m.ScenarioPlan(
                operations=[],
                expected_changed_parts={"ppt/slides/slide1.xml"},
                expected_markers={self.m.TEXT_MARKER: "ppt/slides/slide1.xml"},
            )
            report = {"changed_parts": ["ppt/slides/slide1.xml"]}
            res = self.m.check_structure(inp, out, plan, report)
            self.assertEqual(res.status, "pass", res.details)

    def test_unrelated_part_byte_change_fails(self):
        import tempfile

        with tempfile.TemporaryDirectory() as t:
            tmp = pathlib.Path(t)
            inp = self._write(tmp, "in.pptx", {
                "ppt/slides/slide1.xml": b"<old/>",
                "ppt/slides/slide2.xml": b"<keep/>",
            })
            out = self._write(tmp, "out.pptx", {
                "ppt/slides/slide1.xml": b"<new>PPTX_COMPOSE_E2E_EDIT_MARKER</new>",
                "ppt/slides/slide2.xml": b"<TAMPERED/>",
            })
            plan = self.m.ScenarioPlan(
                operations=[],
                expected_changed_parts={"ppt/slides/slide1.xml"},
                expected_markers={self.m.TEXT_MARKER: "ppt/slides/slide1.xml"},
            )
            report = {"changed_parts": ["ppt/slides/slide1.xml"]}
            res = self.m.check_structure(inp, out, plan, report)
            self.assertEqual(res.status, "fail")
            self.assertTrue(any("preservation violated" in d for d in res.details))

    def test_missing_marker_fails(self):
        import tempfile

        with tempfile.TemporaryDirectory() as t:
            tmp = pathlib.Path(t)
            inp = self._write(tmp, "in.pptx", {"ppt/slides/slide1.xml": b"<old/>"})
            out = self._write(tmp, "out.pptx", {"ppt/slides/slide1.xml": b"<new/>"})
            plan = self.m.ScenarioPlan(
                operations=[],
                expected_changed_parts={"ppt/slides/slide1.xml"},
                expected_markers={self.m.TEXT_MARKER: "ppt/slides/slide1.xml"},
            )
            report = {"changed_parts": ["ppt/slides/slide1.xml"]}
            res = self.m.check_structure(inp, out, plan, report)
            self.assertEqual(res.status, "fail")
            self.assertTrue(any("not found" in d for d in res.details))

    def test_added_media_part_is_allowed_change(self):
        import tempfile

        with tempfile.TemporaryDirectory() as t:
            tmp = pathlib.Path(t)
            inp = self._write(tmp, "in.pptx", {
                "ppt/slides/slide1.xml": b"<s/>",
                "ppt/slides/_rels/slide1.xml.rels": b"<rels/>",
                "[Content_Types].xml": b"<ct/>",
            })
            out = self._write(tmp, "out.pptx", {
                "ppt/slides/slide1.xml": b"<s2/>",
                "ppt/slides/_rels/slide1.xml.rels": b"<rels2/>",
                "[Content_Types].xml": b"<ct2/>",
                "ppt/media/image2.png": b"\x89PNG-new",
            })
            plan = self.m.ScenarioPlan(
                operations=[],
                expected_changed_parts={
                    "ppt/slides/slide1.xml",
                    "ppt/slides/_rels/slide1.xml.rels",
                },
            )
            report = {"changed_parts": [
                "ppt/slides/slide1.xml",
                "ppt/slides/_rels/slide1.xml.rels",
                "[Content_Types].xml",
                "ppt/media/image2.png",
            ]}
            res = self.m.check_structure(inp, out, plan, report)
            self.assertEqual(res.status, "pass", res.details)

    def test_removed_part_fails(self):
        import tempfile

        with tempfile.TemporaryDirectory() as t:
            tmp = pathlib.Path(t)
            inp = self._write(tmp, "in.pptx", {
                "ppt/slides/slide1.xml": b"<s/>",
                "ppt/media/image1.png": b"keepme",
            })
            out = self._write(tmp, "out.pptx", {
                "ppt/slides/slide1.xml": b"<s2/>",
            })
            plan = self.m.ScenarioPlan(
                operations=[], expected_changed_parts={"ppt/slides/slide1.xml"})
            report = {"changed_parts": ["ppt/slides/slide1.xml"]}
            res = self.m.check_structure(inp, out, plan, report)
            self.assertEqual(res.status, "fail")
            self.assertTrue(any("removed" in d for d in res.details))


class ValidationAndOpinionTest(unittest.TestCase):
    def setUp(self):
        self.m = load_edit_module()

    def test_validation_valid_passes(self):
        res = self.m.check_validation({"validation": {"status": "valid", "errors": 0}})
        self.assertEqual(res.status, "pass")

    def test_validation_with_errors_fails(self):
        res = self.m.check_validation({"validation": {"status": "valid", "errors": 2}})
        self.assertEqual(res.status, "fail")

    def test_validation_bad_status_fails(self):
        res = self.m.check_validation({"validation": {"status": "invalid", "errors": 0}})
        self.assertEqual(res.status, "fail")

    def _report(self, status, **kw):
        skipped = self.m.rt.ComparisonResult("skipped", [])
        passed = self.m.rt.ComparisonResult("pass")
        defaults = dict(apply=passed, structure=passed, validation=passed, visual=skipped)
        defaults.update(kw)
        return self.m.EditReport(
            scenario="replace_text", fixture="f.pptx", status=status,
            output_pptx="o", patch_path="p", report_path="r", log_path="l",
            **defaults,
        )

    def test_opinion_passes_when_visual_skipped(self):
        opinion = self.m.form_opinion([self._report("pass")])
        self.assertEqual(opinion.status, "pass")
        self.assertIn("inconclusive", " ".join(opinion.reasons))

    def test_opinion_fails_on_structure_failure(self):
        bad = self.m.rt.ComparisonResult("fail", ["unrelated parts changed bytes"])
        opinion = self.m.form_opinion([self._report("fail", structure=bad)])
        self.assertEqual(opinion.status, "fail")
        self.assertIn("structure failed", " ".join(opinion.reasons))

    def test_opinion_fails_on_visual_regression(self):
        bad = self.m.rt.ComparisonResult("fail", ["slide 1 visual RMS exceeds threshold"])
        opinion = self.m.form_opinion([self._report("fail", visual=bad)])
        self.assertEqual(opinion.status, "fail")
        self.assertIn("visual failed", " ".join(opinion.reasons))


class NegativeAndDefectFilingTest(unittest.TestCase):
    def setUp(self):
        self.m = load_edit_module()

    def test_json_error_codes_reads_json_errors_stderr_envelope(self):
        result = subprocess.CompletedProcess(
            args=["pptx-compose"],
            returncode=22,
            stdout="",
            stderr='{"schema":"pptx-compose.error.v1","error":{"code":"selector_guard_failed"}}\n',
        )

        self.assertEqual(self.m.json_error_codes(result), ["selector_guard_failed"])

    def test_expected_error_code_assertion_fails_on_missing_or_unexpected_code(self):
        result = subprocess.CompletedProcess(
            args=["pptx-compose"],
            returncode=24,
            stdout="",
            stderr='{"schema":"pptx-compose.error.v1","error":{"code":"unsupported_edit"}}\n',
        )

        ok = self.m.check_expected_error_codes(result, ("unsupported_edit",))
        bad = self.m.check_expected_error_codes(result, ("selector_guard_failed",))

        self.assertEqual(ok.status, "pass")
        self.assertEqual(bad.status, "fail")
        self.assertIn("missing expected error code", "\n".join(bad.details))
        self.assertIn("unexpected error code", "\n".join(bad.details))

    def test_negative_scenario_requires_expected_json_error_code(self):
        import tempfile

        with tempfile.TemporaryDirectory() as t:
            tmp = pathlib.Path(t)
            fixture = tmp / "fixture.pptx"
            fixture.write_bytes(b"not used because helpers are mocked")
            cli = tmp / "pptx-compose"
            scenario = self.m.EditScenario(
                "stale_guard",
                str(fixture),
                1,
                lambda view, ctx: self.m.ScenarioPlan(operations=[{"op": "replace_text"}]),
                expect_apply_failure=True,
                expected_error_codes=("selector_guard_failed",),
            )
            with (
                mock.patch.object(self.m.rt, "resolve_fixture", return_value=fixture),
                mock.patch.object(self.m, "inspect_slide", return_value=sample_view()),
                mock.patch.object(self.m, "apply_patch") as apply_patch,
            ):
                apply_patch.return_value = subprocess.CompletedProcess(
                    args=["pptx-compose"],
                    returncode=22,
                    stdout="",
                    stderr='{"schema":"pptx-compose.error.v1","error":{"code":"selector_guard_failed"}}\n',
                )
                report = self.m.run_scenario(
                    REPO_ROOT,
                    tmp,
                    tmp,
                    cli,
                    scenario,
                    {},
                    visual_threshold=8.0,
                )

        self.assertEqual(report.status, "pass")
        self.assertEqual(report.apply.status, "pass")
        self.assertIn("selector_guard_failed", "\n".join(report.apply.details))

    def test_negative_scenario_fails_when_error_code_does_not_match(self):
        import tempfile

        with tempfile.TemporaryDirectory() as t:
            tmp = pathlib.Path(t)
            fixture = tmp / "fixture.pptx"
            fixture.write_bytes(b"not used because helpers are mocked")
            cli = tmp / "pptx-compose"
            scenario = self.m.EditScenario(
                "stale_guard",
                str(fixture),
                1,
                lambda view, ctx: self.m.ScenarioPlan(operations=[{"op": "replace_text"}]),
                expect_apply_failure=True,
                expected_error_codes=("selector_guard_failed",),
            )
            with (
                mock.patch.object(self.m.rt, "resolve_fixture", return_value=fixture),
                mock.patch.object(self.m, "inspect_slide", return_value=sample_view()),
                mock.patch.object(self.m, "apply_patch") as apply_patch,
            ):
                apply_patch.return_value = subprocess.CompletedProcess(
                    args=["pptx-compose"],
                    returncode=24,
                    stdout="",
                    stderr='{"schema":"pptx-compose.error.v1","error":{"code":"unsupported_edit"}}\n',
                )
                report = self.m.run_scenario(
                    REPO_ROOT,
                    tmp,
                    tmp,
                    cli,
                    scenario,
                    {},
                    visual_threshold=8.0,
                )

        self.assertEqual(report.status, "fail")
        self.assertEqual(report.apply.status, "fail")
        self.assertIn("missing expected error code", "\n".join(report.apply.details))

    def test_issue_description_contains_edit_artifacts_and_failures(self):
        report = self.m.EditReport(
            scenario="replace_text",
            fixture="fixtures/example.pptx",
            status="fail",
            apply=self.m.rt.ComparisonResult("fail", ["apply failed"]),
            structure=self.m.rt.ComparisonResult("skipped", ["apply failed"]),
            validation=self.m.rt.ComparisonResult("skipped", []),
            visual=self.m.rt.ComparisonResult("skipped", []),
            output_pptx="/tmp/out.pptx",
            patch_path="/tmp/patch.json",
            report_path="/tmp/report.json",
            log_path="/tmp/edit.log",
        )

        description = self.m.issue_description(report)

        self.assertIn("replace_text", description)
        self.assertIn("fixtures/example.pptx", description)
        self.assertIn("apply failed", description)
        self.assertIn("/tmp/edit.log", description)

    def test_file_bead_creates_deduped_edit_defect(self):
        report = self.m.EditReport(
            scenario="replace_text",
            fixture="fixtures/example.pptx",
            status="fail",
            apply=self.m.rt.ComparisonResult("fail", ["apply failed"]),
            structure=self.m.rt.ComparisonResult("skipped", []),
            validation=self.m.rt.ComparisonResult("skipped", []),
            visual=self.m.rt.ComparisonResult("skipped", []),
            output_pptx="/tmp/out.pptx",
            patch_path="/tmp/patch.json",
            report_path="/tmp/report.json",
            log_path="/tmp/edit.log",
        )
        completed = subprocess.CompletedProcess(args=["bd"], returncode=0, stdout="created\n", stderr="")

        with (
            mock.patch.object(self.m.shutil, "which", return_value="/usr/bin/bd"),
            mock.patch.object(self.m.rt, "open_issue_exists", return_value=False),
            mock.patch.object(self.m.subprocess, "run", return_value=completed) as run,
        ):
            self.m.file_bead(REPO_ROOT, report)

        command = run.call_args.args[0]
        self.assertIn("bd", command)
        self.assertIn("defect:edit-e2e", " ".join(command))
        self.assertIn("--description", command)

    def test_file_bead_skips_existing_open_defect(self):
        report = self.m.EditReport(
            scenario="replace_text",
            fixture="fixtures/example.pptx",
            status="fail",
            apply=self.m.rt.ComparisonResult("fail", ["apply failed"]),
            structure=self.m.rt.ComparisonResult("skipped", []),
            validation=self.m.rt.ComparisonResult("skipped", []),
            visual=self.m.rt.ComparisonResult("skipped", []),
            output_pptx="/tmp/out.pptx",
            patch_path="/tmp/patch.json",
            report_path="/tmp/report.json",
            log_path="/tmp/edit.log",
        )

        with (
            mock.patch.object(self.m.shutil, "which", return_value="/usr/bin/bd"),
            mock.patch.object(self.m.rt, "open_issue_exists", return_value=True),
            mock.patch.object(self.m.subprocess, "run") as run,
        ):
            self.m.file_bead(REPO_ROOT, report)

        run.assert_not_called()


class LoopWiringTest(unittest.TestCase):
    def test_loop_invokes_edit_e2e_under_include_tests(self):
        loop = LOOP.read_text()
        self.assertIn("pptx_edit_e2e.py", loop)
        self.assertIn("edit-e2e", loop)


@unittest.skipUnless(CLI_BIN.exists(),
                     f"pptx-compose debug binary not built at {CLI_BIN}")
class EditRoundTripIntegrationTest(unittest.TestCase):
    """End-to-end: inspect real fixture, apply each V1 op, assert invariants.

    Skipped automatically when the Rust debug binary has not been built. The
    visual stage degrades to 'skipped' (inconclusive) without LibreOffice.
    """

    @classmethod
    def setUpClass(cls):
        cls.m = load_edit_module()
        import tempfile
        cls._tmp = tempfile.TemporaryDirectory()
        cls.work_dir = pathlib.Path(cls._tmp.name)
        cls.log_dir = cls.work_dir / "logs"
        cls.log_dir.mkdir(parents=True, exist_ok=True)
        cls.cli = CLI_BIN
        cls.media_ctx = cls.m.prepare_media_context(cls.work_dir)

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _run(self, name):
        scenario = next(s for s in self.m.DEFAULT_SCENARIOS if s.name == name)
        return self.m.run_scenario(
            REPO_ROOT, self.work_dir, self.log_dir, self.cli, scenario,
            self.media_ctx, visual_threshold=8.0,
        )

    def _assert_ok(self, report):
        # A scenario may legitimately skip if a fixture lacks a target; that is
        # informational, not a failure.
        if report.status == "skipped":
            self.skipTest(f"{report.scenario} skipped: {report.apply.details}")
        self.assertEqual(report.apply.status, "pass",
                         f"apply: {report.apply.details}; log={report.log_path}")
        self.assertEqual(report.structure.status, "pass",
                         f"structure: {report.structure.details}")
        self.assertEqual(report.validation.status, "pass",
                         f"validation: {report.validation.details}")
        # Visual is pass (faithful) or skipped (no LibreOffice); never silently fail.
        self.assertIn(report.visual.status, ("pass", "skipped"),
                      f"visual: {report.visual.details}")

    def test_replace_text(self):
        self._assert_ok(self._run("replace_text"))

    def test_set_alt_text(self):
        self._assert_ok(self._run("set_alt_text"))

    def test_add_text_box(self):
        self._assert_ok(self._run("add_text_box"))

    def test_move_resize_element(self):
        self._assert_ok(self._run("move_resize_element"))

    def test_add_image(self):
        self._assert_ok(self._run("add_image"))

    def test_replace_image(self):
        self._assert_ok(self._run("replace_image"))


if __name__ == "__main__":
    unittest.main()
