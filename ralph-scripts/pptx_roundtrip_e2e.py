#!/usr/bin/env python3
"""PPTX no-edit V1 E2E round-trip checker for loop.sh.

The runner exercises the V1 agent CLI path, compares input/output at the
package/XML/media level, optionally performs a visual render comparison when
LibreOffice + pdftoppm + Pillow are installed, and files consolidated Beads for
detected defects.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import textwrap
import tomllib
import zipfile
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Iterable


DEFECT_LABEL = "defect:roundtrip-e2e"


@dataclass
class ComparisonResult:
    status: str
    details: list[str] = field(default_factory=list)


@dataclass
class FixtureReport:
    fixture: str
    status: str
    commands: list[str]
    xml: ComparisonResult
    media: ComparisonResult
    visual: ComparisonResult
    validation: ComparisonResult
    output_pptx: str
    view_path: str
    patch_path: str
    dry_run_report_path: str
    log_path: str


@dataclass
class Opinion:
    status: str
    reasons: list[str]


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    project_dir = args.project_dir.resolve()
    work_dir = args.work_dir.resolve() if args.work_dir else project_dir / ".ralph-roundtrip-e2e"
    work_dir.mkdir(parents=True, exist_ok=True)

    fixtures = args.fixture or manifest_roundtrip_fixtures(project_dir)
    if not fixtures:
        print("No roundtrip fixtures found.")
        return 0

    log_dir = work_dir / "logs"
    log_dir.mkdir(parents=True, exist_ok=True)
    cli = ensure_cli(project_dir, log_dir)

    reports = [
        run_fixture(project_dir, work_dir, log_dir, cli, fixture, args.visual_threshold)
        for fixture in fixtures
    ]
    opinion = form_opinion(reports)

    summary_path = work_dir / "roundtrip-summary.json"
    summary = {
        "schema": "pptx-compose.roundtrip-e2e.v1",
        "status": opinion.status,
        "opinion": asdict(opinion),
        "reports": [asdict(report) for report in reports],
    }
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")

    print(f"Round-trip E2E opinion: {opinion.status}")
    for reason in opinion.reasons:
        print(f"  - {reason}")
    print(f"Report: {summary_path}")

    if args.file_beads and opinion.status == "fail":
        for report in reports:
            if report.status == "fail":
                file_bead(project_dir, report)

    return 1 if opinion.status == "fail" else 0


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project-dir", type=Path, default=Path.cwd())
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument(
        "--fixture",
        action="append",
        default=[],
        help="Fixture path relative to fixtures/ or project root. May be repeated.",
    )
    parser.add_argument("--file-beads", action="store_true")
    parser.add_argument("--visual-threshold", type=float, default=0.5)
    return parser.parse_args(argv)


def ensure_cli(project_dir: Path, log_dir: Path) -> Path:
    cli = project_dir / "target" / "debug" / "pptx-compose"
    if cli.exists():
        return cli

    build_log = log_dir / "cargo-build-cli.log"
    command = ["cargo", "build", "-p", "pptx-compose-cli", "--bin", "pptx-compose"]
    result = run_command(project_dir, command, build_log)
    if result.returncode != 0:
        raise SystemExit(f"Could not build pptx-compose CLI; see {build_log}")
    return cli


def manifest_roundtrip_fixtures(project_dir: Path) -> list[str]:
    manifest = project_dir / "fixtures" / "manifest.toml"
    if not manifest.exists():
        return ["fixtures/legacy/sample.pptx"]
    data = tomllib.loads(manifest.read_text())
    fixtures = []
    for entry in data.get("entries", []):
        invariants = set(entry.get("invariants", []))
        if "malformed" not in invariants:
            fixtures.append(str(Path("fixtures") / entry["path"]))
    return fixtures or ["fixtures/legacy/sample.pptx"]


def run_fixture(
    project_dir: Path,
    work_dir: Path,
    log_dir: Path,
    cli: Path,
    fixture: str,
    visual_threshold: float,
) -> FixtureReport:
    input_path = resolve_fixture(project_dir, fixture)
    fixture_key = safe_name(fixture)
    fixture_dir = work_dir / fixture_key
    if fixture_dir.exists():
        shutil.rmtree(fixture_dir)
    fixture_dir.mkdir(parents=True)

    view_path = fixture_dir / "inspect.json"
    patch_path = fixture_dir / "noop.patch.json"
    dry_run_report_path = fixture_dir / "dry-run.report.json"
    apply_report_path = fixture_dir / "apply.report.json"
    output_path = fixture_dir / "roundtrip.pptx"
    validation_path = fixture_dir / "validation.json"
    log_path = log_dir / f"{fixture_key}.log"
    commands = [
        f"{cli} --json-errors --temp-dir {work_dir} inspect {input_path} --format agent-json --output {view_path}",
        f"{cli} --json-errors --temp-dir {work_dir} apply {input_path} {patch_path} --dry-run --report {dry_run_report_path}",
        f"{cli} --json-errors --temp-dir {work_dir} apply {input_path} {patch_path} --output {output_path} --report {apply_report_path}",
        f"{cli} --json-errors --temp-dir {work_dir} validate {output_path} --report {validation_path}",
    ]

    with log_path.open("w") as log:
        log.write(f"fixture={fixture}\ninput={input_path}\noutput={output_path}\n")

    conversion = prepare_no_edit_patch(project_dir, cli, work_dir, input_path, view_path, patch_path, log_path)
    if conversion.status == "pass":
        dry_run = run_command(
            project_dir,
            [
                str(cli), "--json-errors", "--temp-dir", str(work_dir),
                "apply", str(input_path), str(patch_path),
                "--dry-run", "--report", str(dry_run_report_path),
            ],
            log_path,
        )
        if dry_run.returncode != 0:
            conversion = ComparisonResult("fail", [f"dry-run apply command failed; see {log_path}"])
    if conversion.status == "pass":
        apply = run_command(
            project_dir,
            [
                str(cli), "--json-errors", "--temp-dir", str(work_dir),
                "apply", str(input_path), str(patch_path),
                "--output", str(output_path), "--report", str(apply_report_path),
            ],
            log_path,
        )
        if apply.returncode != 0:
            conversion = ComparisonResult("fail", [f"apply command failed; see {log_path}"])

    if conversion.status == "fail":
        return FixtureReport(
            fixture=fixture,
            status="fail",
            commands=commands,
            xml=conversion,
            media=ComparisonResult("skipped", ["V1 no-edit write failed"]),
            visual=ComparisonResult("skipped", ["V1 no-edit write failed"]),
            validation=ComparisonResult("skipped", ["V1 no-edit write failed"]),
            output_pptx=str(output_path),
            view_path=str(view_path),
            patch_path=str(patch_path),
            dry_run_report_path=str(dry_run_report_path),
            log_path=str(log_path),
        )

    validation = validate_output(project_dir, cli, work_dir, output_path, validation_path, log_path)
    xml = compare_xml_and_package(input_path, output_path)
    media = compare_media(input_path, output_path)
    visual = compare_visual(input_path, output_path, fixture_dir / "visual", visual_threshold, log_path)
    status = "fail" if any(result.status == "fail" for result in (validation, xml, media, visual)) else "pass"

    return FixtureReport(
        fixture=fixture,
        status=status,
        commands=commands,
        xml=xml,
        media=media,
        visual=visual,
        validation=validation,
        output_pptx=str(output_path),
        view_path=str(view_path),
        patch_path=str(patch_path),
        dry_run_report_path=str(dry_run_report_path),
        log_path=str(log_path),
    )


def prepare_no_edit_patch(
    project_dir: Path,
    cli: Path,
    work_dir: Path,
    input_path: Path,
    view_path: Path,
    patch_path: Path,
    log_path: Path,
) -> ComparisonResult:
    result = run_command(
        project_dir,
        [
            str(cli), "--json-errors", "--temp-dir", str(work_dir),
            "inspect", str(input_path),
            "--format", "agent-json", "--output", str(view_path),
        ],
        log_path,
    )
    if result.returncode != 0:
        return ComparisonResult("fail", [f"inspect command failed; see {log_path}"])
    try:
        view = json.loads(view_path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        return ComparisonResult("fail", [f"inspect view could not be read: {exc}"])
    try:
        patch = {
            "schema": "pptx-compose.patch.v1",
            "version": 1,
            "document_id": view["document_id"],
            "base_revision": view["revision"],
            "client_request_id": "pptx-compose-roundtrip-e2e-no-edit",
            "operations": [],
        }
    except KeyError as exc:
        return ComparisonResult("fail", [f"inspect view omitted required patch guard: {exc}"])
    patch_path.write_text(json.dumps(patch, indent=2, sort_keys=True) + "\n")
    return ComparisonResult("pass")


def validate_output(project_dir: Path, cli: Path, work_dir: Path, output_path: Path, validation_path: Path, log_path: Path) -> ComparisonResult:
    result = run_command(
        project_dir,
        [
            str(cli), "--json-errors", "--temp-dir", str(work_dir),
            "validate", str(output_path), "--report", str(validation_path),
        ],
        log_path,
    )
    if result.returncode != 0:
        return ComparisonResult("fail", [f"validate failed; see {log_path}"])
    try:
        report = json.loads(validation_path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        return ComparisonResult("fail", [f"validation report could not be read: {exc}"])
    status = report.get("status")
    if status not in {"valid", "pass", "ok"}:
        return ComparisonResult("fail", [f"validation status was {status!r}"])
    return ComparisonResult("pass")


def compare_xml_and_package(input_path: Path, output_path: Path) -> ComparisonResult:
    input_entries = zip_entries(input_path)
    output_entries = zip_entries(output_path)
    details = []

    missing = sorted(set(input_entries) - set(output_entries))
    extra = sorted(set(output_entries) - set(input_entries))
    details.extend(f"missing package entry: {name}" for name in missing[:20])
    details.extend(f"extra package entry: {name}" for name in extra[:20])

    for name in sorted(set(input_entries) & set(output_entries)):
        if is_xml_part(name) and input_entries[name] != output_entries[name]:
            details.append(f"XML part differs: {name}")
        if len(details) >= 50:
            details.append("additional XML/package differences omitted")
            break

    return ComparisonResult("fail" if details else "pass", details)


def compare_media(input_path: Path, output_path: Path) -> ComparisonResult:
    input_entries = zip_entries(input_path)
    output_entries = zip_entries(output_path)
    details = []
    input_media = {name: value for name, value in input_entries.items() if name.startswith("ppt/media/")}
    output_media = {name: value for name, value in output_entries.items() if name.startswith("ppt/media/")}
    for name in sorted(set(input_media) | set(output_media)):
        if name not in output_media:
            details.append(f"missing media entry: {name}")
        elif name not in input_media:
            details.append(f"extra media entry: {name}")
        elif input_media[name] != output_media[name]:
            details.append(f"media bytes differ: {name}")
    return ComparisonResult("fail" if details else "pass", details[:50])


def compare_visual(input_path: Path, output_path: Path, visual_dir: Path, threshold: float, log_path: Path) -> ComparisonResult:
    soffice = shutil.which("soffice") or shutil.which("libreoffice")
    pdftoppm = shutil.which("pdftoppm")
    try:
        from PIL import Image, ImageChops, ImageStat  # type: ignore
    except ImportError:
        return ComparisonResult("skipped", ["Pillow is not installed"])
    if not soffice:
        return ComparisonResult("skipped", ["LibreOffice/soffice is not installed"])
    if not pdftoppm:
        return ComparisonResult("skipped", ["pdftoppm is not installed"])

    visual_dir.mkdir(parents=True, exist_ok=True)
    input_pdf = render_pdf(input_path, visual_dir / "input", soffice, log_path)
    if input_pdf is None:
        return ComparisonResult("skipped", [f"input visual PDF render failed; see {log_path}"])

    output_pdf = render_pdf(output_path, visual_dir / "output", soffice, log_path)
    if output_pdf is None:
        return ComparisonResult("fail", [f"output visual PDF render failed; see {log_path}"])

    input_images = render_images(input_pdf, visual_dir / "input-slide", pdftoppm, log_path)
    if not input_images:
        return ComparisonResult("skipped", [f"input visual image render failed; see {log_path}"])

    output_images = render_images(output_pdf, visual_dir / "output-slide", pdftoppm, log_path)
    if not output_images:
        return ComparisonResult("fail", [f"output visual image render failed; see {log_path}"])
    if len(input_images) != len(output_images):
        return ComparisonResult("fail", [f"visual slide count differs: {len(input_images)} != {len(output_images)}"])

    details = []
    for index, (left, right) in enumerate(zip(input_images, output_images), start=1):
        with Image.open(left) as left_image, Image.open(right) as right_image:
            if left_image.size != right_image.size:
                details.append(f"slide {index} visual size differs: {left_image.size} != {right_image.size}")
                continue
            diff = ImageChops.difference(left_image.convert("RGB"), right_image.convert("RGB"))
            rms = sum(value**2 for value in ImageStat.Stat(diff).rms) ** 0.5
            if rms > threshold:
                details.append(f"slide {index} visual RMS {rms:.3f} exceeds threshold {threshold:.3f}")
    return ComparisonResult("fail" if details else "pass", details)


def render_pdf(pptx: Path, out_dir: Path, soffice: str, log_path: Path) -> Path | None:
    out_dir.mkdir(parents=True, exist_ok=True)
    result = run_command(Path.cwd(), [soffice, "--headless", "--convert-to", "pdf", "--outdir", str(out_dir), str(pptx)], log_path)
    if result.returncode != 0:
        return None
    pdfs = sorted(out_dir.glob("*.pdf"))
    return pdfs[0] if pdfs else None


def render_images(pdf: Path, prefix: Path, pdftoppm: str, log_path: Path) -> list[Path]:
    result = run_command(Path.cwd(), [pdftoppm, "-png", "-r", "120", str(pdf), str(prefix)], log_path)
    if result.returncode != 0:
        return []
    return sorted(prefix.parent.glob(prefix.name + "-*.png"))


def form_opinion(reports: Iterable[FixtureReport]) -> Opinion:
    reasons = []
    failures = 0
    for report in reports:
        if report.validation.status == "fail":
            failures += 1
            reasons.append(f"{report.fixture}: validation failed")
        if report.xml.status == "fail":
            failures += 1
            reasons.append(f"{report.fixture}: XML/package comparison failed")
        if report.media.status == "fail":
            failures += 1
            reasons.append(f"{report.fixture}: media byte comparison failed")
        if report.visual.status == "fail":
            failures += 1
            reasons.append(f"{report.fixture}: visual comparison failed")
    if failures:
        return Opinion("fail", reasons)
    return Opinion("pass", ["All required round-trip signals passed; skipped visual checks are informational."])


def issue_description(report: FixtureReport) -> str:
    details = []
    for label, result in (
        ("Validation", report.validation),
        ("XML/package comparison", report.xml),
        ("Media comparison", report.media),
        ("Visual comparison", report.visual),
    ):
        rendered = "\n".join(f"- {item}" for item in result.details) or "- no details"
        details.append(f"{label}: {result.status}\n{rendered}")

    commands = "\n".join(f"- `{command}`" for command in report.commands)
    return textwrap.dedent(
        f"""
        Why this exists:
        The loop round-trip E2E check detected a V1 no-edit write regression for `{report.fixture}`.

        What failed:
        {chr(10).join(details)}

        Reproduction commands:
        {commands}

        Artifacts:
        - Output PPTX: `{report.output_pptx}`
        - Inspect view: `{report.view_path}`
        - No-edit patch: `{report.patch_path}`
        - Dry-run report: `{report.dry_run_report_path}`
        - Log: `{report.log_path}`

        What needs to be done:
        Inspect the log and artifacts, identify whether inspect, patch apply, package writing, validation, or rendering exposed the defect, then fix the smallest code path that restores a clean round trip.
        """
    ).strip()


def file_bead(project_dir: Path, report: FixtureReport) -> None:
    if not shutil.which("bd"):
        print(f"  ⚠ bd not found; could not file round-trip defect for {report.fixture}")
        return
    title = f"Round-trip E2E detected issue in {report.fixture}"
    if open_issue_exists(project_dir, title):
        print(f"  Existing round-trip defect bead found for {report.fixture}; not duplicating")
        return
    command = [
        "bd",
        "create",
        "--title",
        title,
        "--type",
        "bug",
        "--priority",
        "1",
        "--labels",
        f"testing,tier:task,{DEFECT_LABEL}",
        "--description",
        issue_description(report),
    ]
    result = subprocess.run(command, cwd=project_dir, text=True, capture_output=True, check=False)
    if result.returncode == 0:
        print(result.stdout.strip())
    else:
        print(f"  ⚠ could not file round-trip defect bead: {result.stderr.strip()}")


def open_issue_exists(project_dir: Path, title: str) -> bool:
    result = subprocess.run(
        ["bd", "list", "--status=open", "--limit", "0", "--json"],
        cwd=project_dir,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        return False
    try:
        issues = json.loads(result.stdout)
    except json.JSONDecodeError:
        return False
    return any(issue.get("title") == title for issue in issues)


def zip_entries(path: Path) -> dict[str, bytes]:
    with zipfile.ZipFile(path) as archive:
        return {
            info.filename: archive.read(info.filename)
            for info in archive.infolist()
            if not info.is_dir()
        }


def is_xml_part(name: str) -> bool:
    return name.endswith(".xml") or name.endswith(".rels") or name == "[Content_Types].xml"


def resolve_fixture(project_dir: Path, fixture: str) -> Path:
    path = Path(fixture)
    candidates = [project_dir / path]
    if not fixture.startswith("fixtures/"):
        candidates.append(project_dir / "fixtures" / path)
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise FileNotFoundError(f"fixture not found: {fixture}")


def run_command(cwd: Path, command: list[str], log_path: Path) -> subprocess.CompletedProcess[str]:
    with log_path.open("a") as log:
        log.write("\n$ " + " ".join(command) + "\n")
        result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, check=False)
        if result.stdout:
            log.write("stdout:\n" + result.stdout)
        if result.stderr:
            log.write("stderr:\n" + result.stderr)
        log.write(f"exit={result.returncode}\n")
        return result


def safe_name(value: str) -> str:
    digest = hashlib.sha256(value.encode()).hexdigest()[:8]
    stem = "".join(char if char.isalnum() else "-" for char in value).strip("-")[:80]
    return f"{stem}-{digest}"


if __name__ == "__main__":
    raise SystemExit(main())
