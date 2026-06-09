import importlib.util
import io
import pathlib
import sys
import tempfile
import unittest
import zipfile


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "ralph-scripts" / "translation_fidelity.py"


def load_module():
    spec = importlib.util.spec_from_file_location("translation_fidelity", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def text_xml(*texts):
    runs = "".join(f"<a:r><a:t>{text}</a:t></a:r>" for text in texts)
    return (
        b'<?xml version="1.0" encoding="UTF-8"?>'
        b'<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" '
        b'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">'
        + runs.encode("utf-8")
        + b"</p:sld>"
    )


def chart_xml(supported, unsupported):
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
  xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart>
    <c:title><c:tx><c:rich><a:p><a:r><a:t>{supported}</a:t></a:r></a:p></c:rich></c:tx></c:title>
    <c:plotArea><c:barChart><c:ser><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>{unsupported}</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea>
  </c:chart>
</c:chartSpace>
""".encode("utf-8")


def rels_xml(*relationships):
    body = "".join(
        f'<Relationship Id="{rid}" Type="{typ}" Target="{target}"/>'
        for rid, typ, target in relationships
    )
    return (
        b'<?xml version="1.0" encoding="UTF-8"?>'
        b'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + body.encode("utf-8")
        + b"</Relationships>"
    )


def make_pptx(path, entries):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)


class TranslationFidelityTest(unittest.TestCase):
    def setUp(self):
        self.m = load_module()

    def test_chart_supported_text_fails_separately_from_unsupported_authoring_text(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            orig = root / "orig.pptx"
            trans = root / "trans.pptx"
            entries = {
                "ppt/slides/slide1.xml": text_xml("Slide title"),
                "ppt/slides/_rels/slide1.xml.rels": rels_xml(
                    ("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart", "../charts/chart1.xml")
                ),
                "ppt/charts/chart1.xml": chart_xml("Revenue outlook", "Workbook Label"),
            }
            make_pptx(orig, entries)
            make_pptx(trans, entries)

            slide = self.m.analyze(orig, trans)[0]

            self.assertEqual(slide["charts"]["supported_total_nl"], 1)
            self.assertEqual(slide["charts"]["supported_unchanged"], 1)
            self.assertEqual(slide["charts"]["unsupported_total_nl"], 1)
            self.assertIn("SUPPORTED_CHART_TEXT_UNTRANSLATED", "\n".join(slide["problems"]))
            self.assertIn("UNSUPPORTED_CHART_TEXT_PRESENT", "\n".join(slide["problems"]))

    def test_smartart_data_and_drawing_mirror_are_checked_together(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            orig = root / "orig.pptx"
            trans = root / "trans.pptx"
            base_entries = {
                "ppt/slides/slide1.xml": text_xml("Slide title"),
                "ppt/slides/_rels/slide1.xml.rels": rels_xml(
                    ("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData", "../diagrams/data1.xml"),
                    ("rId2", "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing", "../diagrams/drawing1.xml"),
                ),
            }
            make_pptx(
                orig,
                {
                    **base_entries,
                    "ppt/diagrams/data1.xml": text_xml("Original node"),
                    "ppt/diagrams/drawing1.xml": text_xml("Original node"),
                },
            )
            make_pptx(
                trans,
                {
                    **base_entries,
                    "ppt/diagrams/data1.xml": text_xml("Translated node"),
                    "ppt/diagrams/drawing1.xml": text_xml("Original node"),
                },
            )

            slide = self.m.analyze(orig, trans)[0]

            self.assertEqual(slide["diagrams"]["unchanged"], 0)
            self.assertEqual(slide["diagrams"]["stale_mirror_missing"], 1)
            self.assertEqual(slide["diagrams"]["stale_mirror_source"], 1)
            self.assertIn("STALE_SMARTART_DRAWING_MIRROR", "\n".join(slide["problems"]))

    def test_translated_smartart_data_and_drawing_have_no_supported_or_stale_findings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            orig = root / "orig.pptx"
            trans = root / "trans.pptx"
            base_entries = {
                "ppt/slides/slide1.xml": text_xml("Slide title"),
                "ppt/slides/_rels/slide1.xml.rels": rels_xml(
                    ("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData", "../diagrams/data1.xml"),
                    ("rId2", "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing", "../diagrams/drawing1.xml"),
                ),
            }
            make_pptx(
                orig,
                {
                    **base_entries,
                    "ppt/diagrams/data1.xml": text_xml("Original node"),
                    "ppt/diagrams/drawing1.xml": text_xml("Original node"),
                },
            )
            make_pptx(
                trans,
                {
                    **base_entries,
                    "ppt/diagrams/data1.xml": text_xml("Translated node"),
                    "ppt/diagrams/drawing1.xml": text_xml("Translated node"),
                },
            )

            slide = self.m.analyze(orig, trans)[0]

            self.assertEqual(slide["diagrams"]["unchanged"], 0)
            self.assertEqual(slide["diagrams"]["stale_mirror_missing"], 0)
            self.assertEqual(slide["diagrams"]["stale_mirror_source"], 0)
            self.assertNotIn("SUPPORTED_DIAGRAM_TEXT_UNTRANSLATED", "\n".join(slide["problems"]))
            self.assertNotIn("STALE_SMARTART_DRAWING_MIRROR", "\n".join(slide["problems"]))


if __name__ == "__main__":
    unittest.main()
