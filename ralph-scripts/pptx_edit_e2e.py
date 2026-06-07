#!/usr/bin/env python3
"""PPTX EDIT round-trip E2E checker.

This complements ``pptx_roundtrip_e2e.py`` (which only exercises the no-edit
PPTX -> JSON -> PPTX conversion path). Here we drive the *real* V1 agent edit
surface through the CLI:

    pptx-compose inspect <input> --format agent-json --detail full --slides N
    pptx-compose apply  <input> <patch.json> [--media REF=PATH] --output <out>

For each fixture + operation we:

1. Inspect a slide to discover a real, editable target (element id, slide id,
   text/fingerprint guards) instead of hard-coding ids.
2. Build a schema-valid ``pptx-compose.patch.v1`` patch carrying the
   ``document_id``/``base_revision`` guards the CLI reported.
3. ``apply`` the patch and read the patch report (``changed_parts``,
   per-operation status, embedded validation).
4. Assert the *targeted* part(s) actually changed (the edit took effect and a
   recognizable marker is present in the touched XML) while every *unrelated*
   part is byte-identical to the input package. This is the core invariant from
   CLAUDE.md/specs/050: edits are confined to a declared dirty set; untouched
   parts keep their original bytes.
5. Run the shared visual stage (soffice + pdftoppm + Pillow) so layout
   regressions a byte/structure check cannot see are caught. The visual stage
   degrades to ``inconclusive`` when LibreOffice is absent (the known
   synthetic-fixture gotcha) so missing tooling never produces a false failure.

Negative scenarios (``expect_apply_failure``) assert the CLI refuses the edit
(non-zero exit, JSON error, no output written) rather than emitting corrupt
bytes -- the ``unsupported_edit`` / ``selector_guard_failed`` escape hatch.

The runner imports the package/visual comparison helpers from
``pptx_roundtrip_e2e`` so there is a single implementation of the
LibreOffice/pdftoppm rendering and graceful-degradation logic.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import shutil
from dataclasses import asdict, dataclass, field
from pathlib import Path

# Import the existing harness so we reuse: run_command, ensure_cli, zip_entries,
# is_xml_part, resolve_fixture, safe_name, compare_visual (+ its graceful
# soffice/pdftoppm/Pillow degradation), ComparisonResult.
_RT_PATH = Path(__file__).with_name("pptx_roundtrip_e2e.py")


def _load_roundtrip_module():
    import sys

    if "pptx_roundtrip_e2e" in sys.modules:
        return sys.modules["pptx_roundtrip_e2e"]
    spec = importlib.util.spec_from_file_location("pptx_roundtrip_e2e", _RT_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    # Register before exec so dataclass forward-ref resolution (Python 3.14)
    # can find the module via cls.__module__ in sys.modules.
    sys.modules["pptx_roundtrip_e2e"] = module
    spec.loader.exec_module(module)
    return module


rt = _load_roundtrip_module()
ComparisonResult = rt.ComparisonResult  # re-exported for callers/tests


EDIT_DEFECT_LABEL = "defect:edit-e2e"

# A stable, easily greppable text marker every text-bearing positive scenario
# writes into the package, so we can assert the edit reached the bytes.
TEXT_MARKER = "PPTX_COMPOSE_E2E_EDIT_MARKER"
ALT_TEXT_MARKER = "PPTX_COMPOSE_E2E_ALT_MARKER"


@dataclass
class EditScenario:
    """A single edit exercise against one fixture.

    ``builder`` is a callable ``(view, ctx) -> ScenarioPlan`` that, given the
    inspected agent view and a mutable context dict, returns the operations to
    apply, any media bindings, and the parts expected to change. Keeping it a
    callable lets a scenario resolve real element ids discovered at runtime.
    """

    name: str
    fixture: str
    slide_number: int
    builder: "callable"
    expect_apply_failure: bool = False
    expected_error_codes: tuple[str, ...] = ()


@dataclass
class ScenarioPlan:
    operations: list[dict]
    media: dict[str, str] = field(default_factory=dict)
    expected_changed_parts: set[str] = field(default_factory=set)
    # Marker -> part it must appear in (verifies the edit reached the bytes).
    expected_markers: dict[str, str] = field(default_factory=dict)
    skip_reason: str | None = None


@dataclass
class EditReport:
    scenario: str
    fixture: str
    status: str  # pass | fail | skipped
    apply: "rt.ComparisonResult"
    structure: "rt.ComparisonResult"
    validation: "rt.ComparisonResult"
    visual: "rt.ComparisonResult"
    output_pptx: str
    patch_path: str
    report_path: str
    log_path: str


@dataclass
class Opinion:
    status: str
    reasons: list[str]


# --------------------------------------------------------------------------- #
# CLI interaction helpers
# --------------------------------------------------------------------------- #


def inspect_slide(project_dir: Path, cli: Path, fixture_path: Path, slide_number: int,
                  out_path: Path, log_path: Path) -> dict | None:
    """Run ``inspect`` for a single slide at full detail and return the view."""
    result = rt.run_command(
        project_dir,
        [
            str(cli), "inspect", str(fixture_path),
            "--format", "agent-json", "--detail", "full",
            "--slides", str(slide_number),
            "--output", str(out_path), "--overwrite",
        ],
        log_path,
    )
    if result.returncode != 0:
        return None
    try:
        return json.loads(out_path.read_text())
    except (OSError, json.JSONDecodeError):
        return None


def editable_text_elements(view: dict) -> list[dict]:
    """All elements whose agent view advertises editable text."""
    out = []
    for slide in view.get("slides", []):
        for el in slide.get("elements", []):
            if el.get("kind") in ("text_box", "shape") and _supports(el, "text"):
                if (el.get("text") or {}).get("plain"):
                    out.append(el)
    return out


def picture_elements(view: dict, editable_only: bool = True) -> list[dict]:
    out = []
    for slide in view.get("slides", []):
        for el in slide.get("elements", []):
            if el.get("kind") == "image":
                if not editable_only or _supports(el, "image"):
                    out.append(el)
    return out


def _supports(el: dict, key: str) -> bool:
    return bool(((el.get("editable") or {}).get(key) or {}).get("supported"))


def first_slide_id(view: dict) -> str | None:
    for slide in view.get("slides", []):
        if slide.get("id"):
            return slide["id"]
    return None


def build_patch(view: dict, operations: list[dict]) -> dict:
    return {
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": view["document_id"],
        "base_revision": view["revision"],
        "client_request_id": "pptx-compose-edit-e2e",
        "operations": operations,
    }


def apply_patch(project_dir: Path, cli: Path, fixture_path: Path, patch_path: Path,
                output_path: Path, report_path: Path, media: dict[str, str],
                log_path: Path) -> "subprocess_result":
    command = [
        str(cli), "--json-errors", "apply", str(fixture_path), str(patch_path),
        "--output", str(output_path), "--report", str(report_path), "--overwrite",
    ]
    for ref, path in media.items():
        command.extend(["--media", f"{ref}={path}"])
    return rt.run_command(project_dir, command, log_path)


# --------------------------------------------------------------------------- #
# Assertions
# --------------------------------------------------------------------------- #


def check_structure(input_path: Path, output_path: Path, plan: ScenarioPlan,
                    report: dict) -> "rt.ComparisonResult":
    """Assert the targeted parts changed and unrelated parts are byte-identical.

    The "allowed to change" set is: the scenario's predeclared parts, plus
    ``[Content_Types].xml`` (media-introducing ops register a new default), plus
    any *newly-added* part the engine allocated and declared in the report's
    ``changed_parts`` (e.g. ``ppt/media/imageN.png`` whose name we cannot
    predict). Everything else must be byte-for-byte preserved.
    """
    details: list[str] = []

    input_entries = rt.zip_entries(input_path)
    output_entries = rt.zip_entries(output_path)

    report_changed = set(report.get("changed_parts", []))
    common = set(input_entries) & set(output_entries)
    byte_changed = {n for n in common if input_entries[n] != output_entries[n]}
    added = set(output_entries) - set(input_entries)
    removed = set(input_entries) - set(output_entries)

    allowed = (
        set(plan.expected_changed_parts)
        | {"[Content_Types].xml"}
        | (added & report_changed)
    )

    # 1. The report's declared changed set must stay within the allowed set and
    #    must include everything the scenario predeclared.
    unexpected_reported = report_changed - allowed
    if unexpected_reported:
        details.append(f"patch report changed unexpected parts: {sorted(unexpected_reported)}")
    missing_reported = set(plan.expected_changed_parts) - report_changed
    if missing_reported:
        details.append(f"patch report omitted expected changed parts: {sorted(missing_reported)}")

    # 2. Every untouched part must be byte-identical (the core preservation
    #    invariant). Only parts in the allowed set may differ.
    untouched_but_changed = byte_changed - allowed
    if untouched_but_changed:
        details.append(
            "unrelated parts changed bytes (preservation violated): "
            f"{sorted(untouched_but_changed)}"
        )

    # 3. The targeted parts must actually differ (or be newly added, for media).
    for part in plan.expected_changed_parts:
        if part in removed:
            details.append(f"expected-changed part was removed: {part}")
        elif part in added:
            continue
        elif part not in byte_changed:
            details.append(f"expected-changed part did not change bytes: {part}")

    # 4. No existing part should silently vanish.
    if removed:
        details.append(f"package entries removed: {sorted(removed)}")

    # 5. Markers prove the edit reached the serialized bytes.
    for marker, part in plan.expected_markers.items():
        target = output_entries.get(part)
        if target is None:
            details.append(f"marker target part missing: {part}")
        elif marker.encode("utf-8") not in target:
            details.append(f"marker {marker!r} not found in {part}")

    return rt.ComparisonResult("fail" if details else "pass", details)


def check_visual_render(input_path: Path, output_path: Path, visual_dir: Path,
                        log_path: Path) -> "rt.ComparisonResult":
    """Edit-aware visual stage.

    Unlike the no-edit round-trip (which expects pixel-identical renders), an
    edit *intentionally* changes pixels (new/replaced text, moved shapes, new
    images). So pixel-identity is the wrong oracle here. Instead we assert the
    edited deck still renders cleanly and produces the *same slide count* as the
    original -- the signal that catches silent corruption, blanked slides, or
    dropped/duplicated slides that byte/structure checks cannot see.

    Degrades to ``skipped`` (inconclusive) when LibreOffice / pdftoppm / Pillow
    are unavailable, so missing tooling never produces a false failure (the
    known synthetic-fixture gotcha).
    """
    soffice = shutil.which("soffice") or shutil.which("libreoffice")
    pdftoppm = shutil.which("pdftoppm")
    try:
        from PIL import Image  # type: ignore  # noqa: F401
    except ImportError:
        return rt.ComparisonResult("skipped", ["Pillow is not installed"])
    if not soffice:
        return rt.ComparisonResult("skipped", ["LibreOffice/soffice is not installed"])
    if not pdftoppm:
        return rt.ComparisonResult("skipped", ["pdftoppm is not installed"])

    visual_dir.mkdir(parents=True, exist_ok=True)

    input_pdf = rt.render_pdf(input_path, visual_dir / "input", soffice, log_path)
    if input_pdf is None:
        return rt.ComparisonResult("skipped", [f"input render failed (cannot baseline); see {log_path}"])
    output_pdf = rt.render_pdf(output_path, visual_dir / "output", soffice, log_path)
    if output_pdf is None:
        return rt.ComparisonResult("fail", [f"edited deck failed to render to PDF; see {log_path}"])

    input_images = rt.render_images(input_pdf, visual_dir / "input-slide", pdftoppm, log_path)
    if not input_images:
        return rt.ComparisonResult("skipped", [f"input image render failed (cannot baseline); see {log_path}"])
    output_images = rt.render_images(output_pdf, visual_dir / "output-slide", pdftoppm, log_path)
    if not output_images:
        return rt.ComparisonResult("fail", [f"edited deck produced no slide images; see {log_path}"])

    if len(input_images) != len(output_images):
        return rt.ComparisonResult(
            "fail",
            [f"slide count changed after edit: {len(input_images)} -> {len(output_images)}"],
        )

    # Guard against an all-white / blank-render corruption: every rendered slide
    # must contain some non-background pixels.
    from PIL import Image, ImageStat  # type: ignore

    details: list[str] = []
    for index, image_path in enumerate(output_images, start=1):
        with Image.open(image_path) as img:
            stat = ImageStat.Stat(img.convert("L"))
            # extrema (min,max); a completely uniform slide is suspicious.
            lo, hi = stat.extrema[0]
            if lo == hi:
                details.append(f"edited slide {index} rendered as a uniform/blank image")
    return rt.ComparisonResult("fail" if details else "pass", details)


def check_validation(report: dict) -> "rt.ComparisonResult":
    validation = report.get("validation") or {}
    status = validation.get("status")
    if status not in {"valid", "pass", "ok"}:
        return rt.ComparisonResult("fail", [f"post-edit validation status was {status!r}"])
    errors = validation.get("errors", 0)
    if errors:
        return rt.ComparisonResult("fail", [f"post-edit validation reported {errors} error(s)"])
    return rt.ComparisonResult("pass")


# --------------------------------------------------------------------------- #
# Scenario builders (each resolves real ids from the inspected view)
# --------------------------------------------------------------------------- #


def build_replace_text(view: dict, ctx: dict) -> ScenarioPlan:
    targets = editable_text_elements(view)
    if not targets:
        return ScenarioPlan([], skip_reason="no editable text element on slide")
    el = targets[0]
    op = {
        "op": "replace_text",
        "operation_id": "op-replace-text",
        "element_id": el["id"],
        "text": f"{TEXT_MARKER} replaced text",
    }
    # Carry guards when present so the edit is matched, not blind.
    guard_hash = (el.get("text") or {}).get("text_hash")
    if guard_hash:
        op["selector"] = {"type": "element_id", "id": el["id"],
                          "guards": {"text_hash": guard_hash,
                                     "fingerprint": el.get("fingerprint")}}
        del op["element_id"]
    return ScenarioPlan(
        operations=[op],
        expected_changed_parts={el["part"]},
        expected_markers={TEXT_MARKER: el["part"]},
    )


def build_set_alt_text(view: dict, ctx: dict) -> ScenarioPlan:
    targets = editable_text_elements(view) or picture_elements(view)
    if not targets:
        return ScenarioPlan([], skip_reason="no element to set alt text on")
    el = targets[0]
    op = {
        "op": "set_alt_text",
        "operation_id": "op-set-alt",
        "element_id": el["id"],
        "alt_text": ALT_TEXT_MARKER,
    }
    return ScenarioPlan(
        operations=[op],
        expected_changed_parts={el["part"]},
        expected_markers={ALT_TEXT_MARKER: el["part"]},
    )


def build_add_text_box(view: dict, ctx: dict) -> ScenarioPlan:
    slide_id = first_slide_id(view)
    if not slide_id:
        return ScenarioPlan([], skip_reason="no slide id available")
    part = view["slides"][0]["part"]
    op = {
        "op": "add_text_box",
        "operation_id": "op-add-textbox",
        "slide_id": slide_id,
        "text": f"{TEXT_MARKER} added box",
        "bounds": {"x": 914400, "y": 914400, "cx": 2743200, "cy": 685800},
    }
    return ScenarioPlan(
        operations=[op],
        expected_changed_parts={part},
        expected_markers={TEXT_MARKER: part},
    )


def build_move_resize(view: dict, ctx: dict) -> ScenarioPlan:
    targets = editable_text_elements(view) or picture_elements(view)
    if not targets:
        return ScenarioPlan([], skip_reason="no movable element on slide")
    el = targets[0]
    bounds = el.get("bounds") or {}
    # Shift + grow so the xfrm bytes definitely change.
    new_bounds = {
        "x": int(bounds.get("x", 914400)) + 100000,
        "y": int(bounds.get("y", 914400)) + 100000,
        "cx": int(bounds.get("cx", 1828800)),
        "cy": int(bounds.get("cy", 685800)),
    }
    op = {
        "op": "move_resize_element",
        "operation_id": "op-move-resize",
        "element_id": el["id"],
        "bounds": new_bounds,
    }
    return ScenarioPlan(operations=[op], expected_changed_parts={el["part"]})


def build_add_image(view: dict, ctx: dict) -> ScenarioPlan:
    slide_id = first_slide_id(view)
    if not slide_id:
        return ScenarioPlan([], skip_reason="no slide id available")
    media_path = ctx.get("png_source")
    if not media_path:
        return ScenarioPlan([], skip_reason="no PNG media source available")
    slide = view["slides"][0]
    slide_part = slide["part"]
    # slide rels part path: ppt/slides/slideN.xml -> ppt/slides/_rels/slideN.xml.rels
    rels_part = _rels_part_for(slide_part)
    op = {
        "op": "add_image",
        "operation_id": "op-add-image",
        "slide_id": slide_id,
        "media_ref": "e2e_added",
        "content_type": "image/png",
        "bounds": {"x": 457200, "y": 457200, "cx": 1828800, "cy": 1371600},
        "alt_text": "e2e added image",
    }
    return ScenarioPlan(
        operations=[op],
        media={"e2e_added": str(media_path)},
        # New media part name is allocated by the engine; we cannot predict it,
        # so we only require the slide xml + rels to change and rely on the
        # report's changed_parts + "no removed parts" + validation for the rest.
        expected_changed_parts={slide_part, rels_part},
    )


def build_replace_image(view: dict, ctx: dict) -> ScenarioPlan:
    pics = picture_elements(view)
    if not pics:
        return ScenarioPlan([], skip_reason="no editable picture on slide")
    media_path = ctx.get("png_replacement")
    if not media_path:
        return ScenarioPlan([], skip_reason="no replacement PNG available")
    el = pics[0]
    rels_part = _rels_part_for(el["part"])
    op = {
        "op": "replace_image",
        "operation_id": "op-replace-image",
        "element_id": el["id"],
        "media_ref": "e2e_replacement",
        "content_type": "image/png",
    }
    return ScenarioPlan(
        operations=[op],
        media={"e2e_replacement": str(media_path)},
        expected_changed_parts={rels_part},
    )


def _rels_part_for(part: str) -> str:
    p = Path(part)
    return str(p.parent / "_rels" / (p.name + ".rels"))


# --------------------------------------------------------------------------- #
# Runner
# --------------------------------------------------------------------------- #


DEFAULT_SCENARIOS = [
    EditScenario("replace_text", "fixtures/real-world/worldbank-cpf-concept-note.pptx", 3, build_replace_text),
    EditScenario("set_alt_text", "fixtures/real-world/worldbank-cpf-concept-note.pptx", 3, build_set_alt_text),
    EditScenario("add_text_box", "fixtures/real-world/worldbank-cpf-concept-note.pptx", 3, build_add_text_box),
    EditScenario("move_resize_element", "fixtures/real-world/worldbank-cpf-concept-note.pptx", 3, build_move_resize),
    EditScenario("add_image", "fixtures/media/image.pptx", 1, build_add_image),
    EditScenario("replace_image", "fixtures/media/image.pptx", 1, build_replace_image),
]


def prepare_media_context(work_dir: Path) -> dict:
    """Stage PNG sources for image ops, reusing real fixture media bytes."""
    ctx: dict = {}
    media_src = work_dir / "media-src"
    media_src.mkdir(parents=True, exist_ok=True)

    import zipfile

    fixture = Path(__file__).resolve().parents[1] / "fixtures" / "media" / "image.pptx"
    if fixture.exists():
        try:
            with zipfile.ZipFile(fixture) as zf:
                pngs = [n for n in zf.namelist() if n.startswith("ppt/media/") and n.endswith(".png")]
                if pngs:
                    src = media_src / "source.png"
                    src.write_bytes(zf.read(pngs[0]))
                    ctx["png_source"] = src
        except (OSError, zipfile.BadZipFile):
            pass

    # A visibly different PNG for replace_image; fall back to the source bytes
    # with a trailing tEXt-ish chunk appended so the bytes definitely differ.
    if "png_source" in ctx:
        repl = media_src / "replacement.png"
        try:
            from PIL import Image  # type: ignore

            Image.new("RGB", (96, 64), (16, 200, 48)).save(repl)
        except Exception:
            data = ctx["png_source"].read_bytes()
            repl.write_bytes(data + b"\x00e2e-replacement-distinct")
        ctx["png_replacement"] = repl

    return ctx


def run_scenario(project_dir: Path, work_dir: Path, log_dir: Path, cli: Path,
                 scenario: EditScenario, media_ctx: dict,
                 visual_threshold: float) -> EditReport:
    fixture_path = rt.resolve_fixture(project_dir, scenario.fixture)
    key = rt.safe_name(f"{scenario.name}-{scenario.fixture}")
    scenario_dir = work_dir / key
    if scenario_dir.exists():
        shutil.rmtree(scenario_dir)
    scenario_dir.mkdir(parents=True)

    log_path = log_dir / f"edit-{key}.log"
    view_path = scenario_dir / "view.json"
    patch_path = scenario_dir / "patch.json"
    output_path = scenario_dir / "edited.pptx"
    report_path = scenario_dir / "report.json"

    def skip(reason: str) -> EditReport:
        skipped = rt.ComparisonResult("skipped", [reason])
        return EditReport(
            scenario=scenario.name, fixture=scenario.fixture, status="skipped",
            apply=skipped, structure=skipped, validation=skipped, visual=skipped,
            output_pptx=str(output_path), patch_path=str(patch_path),
            report_path=str(report_path), log_path=str(log_path),
        )

    view = inspect_slide(project_dir, cli, fixture_path, scenario.slide_number, view_path, log_path)
    if view is None:
        return EditReport(
            scenario=scenario.name, fixture=scenario.fixture, status="fail",
            apply=rt.ComparisonResult("fail", [f"inspect failed; see {log_path}"]),
            structure=rt.ComparisonResult("skipped", ["inspect failed"]),
            validation=rt.ComparisonResult("skipped", ["inspect failed"]),
            visual=rt.ComparisonResult("skipped", ["inspect failed"]),
            output_pptx=str(output_path), patch_path=str(patch_path),
            report_path=str(report_path), log_path=str(log_path),
        )

    plan = scenario.builder(view, media_ctx)
    if plan.skip_reason:
        return skip(plan.skip_reason)

    patch = build_patch(view, plan.operations)
    patch_path.write_text(json.dumps(patch, indent=2) + "\n")

    result = apply_patch(project_dir, cli, fixture_path, patch_path, output_path,
                         report_path, plan.media, log_path)

    if scenario.expect_apply_failure:
        if result.returncode == 0 or output_path.exists():
            return EditReport(
                scenario=scenario.name, fixture=scenario.fixture, status="fail",
                apply=rt.ComparisonResult("fail", ["expected apply to be refused but it succeeded / wrote output"]),
                structure=rt.ComparisonResult("skipped", []),
                validation=rt.ComparisonResult("skipped", []),
                visual=rt.ComparisonResult("skipped", []),
                output_pptx=str(output_path), patch_path=str(patch_path),
                report_path=str(report_path), log_path=str(log_path),
            )
        details = ["apply correctly refused (non-zero exit, no output written)"]
        ok = rt.ComparisonResult("pass", details)
        return EditReport(
            scenario=scenario.name, fixture=scenario.fixture, status="pass",
            apply=ok, structure=rt.ComparisonResult("skipped", ["negative scenario"]),
            validation=rt.ComparisonResult("skipped", ["negative scenario"]),
            visual=rt.ComparisonResult("skipped", ["negative scenario"]),
            output_pptx=str(output_path), patch_path=str(patch_path),
            report_path=str(report_path), log_path=str(log_path),
        )

    if result.returncode != 0:
        return EditReport(
            scenario=scenario.name, fixture=scenario.fixture, status="fail",
            apply=rt.ComparisonResult("fail", [f"apply failed (exit={result.returncode}); see {log_path}"]),
            structure=rt.ComparisonResult("skipped", ["apply failed"]),
            validation=rt.ComparisonResult("skipped", ["apply failed"]),
            visual=rt.ComparisonResult("skipped", ["apply failed"]),
            output_pptx=str(output_path), patch_path=str(patch_path),
            report_path=str(report_path), log_path=str(log_path),
        )

    try:
        report = json.loads(report_path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        return EditReport(
            scenario=scenario.name, fixture=scenario.fixture, status="fail",
            apply=rt.ComparisonResult("fail", [f"patch report unreadable: {exc}"]),
            structure=rt.ComparisonResult("skipped", []),
            validation=rt.ComparisonResult("skipped", []),
            visual=rt.ComparisonResult("skipped", []),
            output_pptx=str(output_path), patch_path=str(patch_path),
            report_path=str(report_path), log_path=str(log_path),
        )

    apply_res = rt.ComparisonResult(
        "pass" if report.get("status") == "applied" else "fail",
        [] if report.get("status") == "applied" else [f"apply report status was {report.get('status')!r}"],
    )
    structure = check_structure(fixture_path, output_path, plan, report)
    validation = check_validation(report)
    visual = check_visual_render(fixture_path, output_path, scenario_dir / "visual",
                                 log_path)

    hard = [r for r in (apply_res, structure, validation) if r.status == "fail"]
    # Visual "fail" is a real regression; visual "skipped" is inconclusive (no
    # LibreOffice) and must not fail the scenario.
    if visual.status == "fail":
        hard.append(visual)
    status = "fail" if hard else "pass"

    return EditReport(
        scenario=scenario.name, fixture=scenario.fixture, status=status,
        apply=apply_res, structure=structure, validation=validation, visual=visual,
        output_pptx=str(output_path), patch_path=str(patch_path),
        report_path=str(report_path), log_path=str(log_path),
    )


def form_opinion(reports: list[EditReport]) -> Opinion:
    reasons: list[str] = []
    for r in reports:
        if r.status == "fail":
            for label, res in (("apply", r.apply), ("structure", r.structure),
                               ("validation", r.validation), ("visual", r.visual)):
                if res.status == "fail":
                    reasons.append(f"{r.scenario} ({r.fixture}): {label} failed -> {'; '.join(res.details) or 'no detail'}")
    if reasons:
        return Opinion("fail", reasons)
    skipped = [r.scenario for r in reports if r.status == "skipped"]
    note = "All edit scenarios passed required signals."
    if skipped:
        note += f" Skipped (no target/tooling): {', '.join(skipped)}."
    if any(r.visual.status == "skipped" for r in reports):
        note += " Visual stage inconclusive where LibreOffice was unavailable."
    return Opinion("pass", [note])


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    project_dir = args.project_dir.resolve()
    work_dir = (args.work_dir.resolve() if args.work_dir
                else project_dir / ".ralph-edit-e2e")
    work_dir.mkdir(parents=True, exist_ok=True)
    log_dir = work_dir / "logs"
    log_dir.mkdir(parents=True, exist_ok=True)

    cli = rt.ensure_cli(project_dir, log_dir)
    media_ctx = prepare_media_context(work_dir)

    scenarios = DEFAULT_SCENARIOS
    if args.scenario:
        wanted = set(args.scenario)
        scenarios = [s for s in scenarios if s.name in wanted]

    reports = [run_scenario(project_dir, work_dir, log_dir, cli, s, media_ctx,
                            args.visual_threshold) for s in scenarios]
    opinion = form_opinion(reports)

    summary_path = work_dir / "edit-summary.json"
    summary_path.write_text(json.dumps({
        "schema": "pptx-compose.edit-e2e.v1",
        "status": opinion.status,
        "opinion": asdict(opinion),
        "reports": [asdict(r) for r in reports],
    }, indent=2, sort_keys=True) + "\n")

    print(f"Edit E2E opinion: {opinion.status}")
    for reason in opinion.reasons:
        print(f"  - {reason}")
    for r in reports:
        print(f"  [{r.status}] {r.scenario}: apply={r.apply.status} "
              f"structure={r.structure.status} validation={r.validation.status} "
              f"visual={r.visual.status}")
    print(f"Report: {summary_path}")

    return 1 if opinion.status == "fail" else 0


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project-dir", type=Path, default=Path.cwd())
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument("--scenario", action="append", default=[],
                        help="Run only the named scenario(s). May be repeated.")
    parser.add_argument("--visual-threshold", type=float, default=8.0)
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(main())
