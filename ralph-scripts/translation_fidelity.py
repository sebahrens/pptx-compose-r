#!/usr/bin/env python3
"""Compare an English PPTX against its translated variants slide-by-slide.

Detects problem classes without python-pptx (pure zipfile + ElementTree):

1. LOST CONTENT   - a slide/part that has fewer text-bearing shapes or runs in the
                    translation than in the original (content silently dropped).
2. UNTRANSLATED   - natural-language runs that are byte-identical between the English
                    original and a translation (the engine could not reach/edit them).
3. SUPPORTED_*    - V1-supported related text, such as visible chart/SmartArt
                    text, remained untranslated.
4. UNSUPPORTED_*  - natural-language text in unsupported chart/SmartArt authoring
                    surfaces. Reported separately from untranslated supported text.
5. STALE_*        - SmartArt data text and rendered drawing mirror diverged.

Output: JSON to stdout, plus a human summary to stderr.
"""
import sys, os, re, json, zipfile
import xml.etree.ElementTree as ET
from collections import defaultdict

A = "http://schemas.openxmlformats.org/drawingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
RELNS = "http://schemas.openxmlformats.org/package/2006/relationships"

def atexts(xml_bytes):
    """All <a:t> string contents, in document order."""
    out = []
    try:
        root = ET.fromstring(xml_bytes)
    except ET.ParseError:
        return out
    for e in root.iter("{%s}t" % A):
        if e.text:
            out.append(e.text)
    return out

def count_breaks(xml_bytes):
    """Visual line breaks in slide text: <a:br/> elements + literal newlines inside <a:t>."""
    try:
        root = ET.fromstring(xml_bytes)
    except ET.ParseError:
        return 0
    br = sum(1 for _ in root.iter("{%s}br" % A))
    nl = sum((e.text or "").count("\n") for e in root.iter("{%s}t" % A))
    return br + nl

def count_paras(xml_bytes):
    try:
        root = ET.fromstring(xml_bytes)
    except ET.ParseError:
        return 0
    return sum(1 for _ in root.iter("{%s}p" % A))

def chart_text_classes(xml_bytes):
    """Classify chart strings as V1-supported visible text or unsupported authoring text."""
    supported = []
    unsupported = []
    try:
        root = ET.fromstring(xml_bytes)
    except ET.ParseError:
        return {"supported": supported, "unsupported": unsupported}
    # Rich text titles/labels are visible chart text.
    for e in root.iter("{%s}t" % A):
        if e.text:
            supported.append(e.text)
    # Cached string values are visible label text when stored in string caches.
    # Natural-language values in numeric caches/formula-backed authoring surfaces
    # are tracked, but do not fail the supported-translation gate.
    C = "http://schemas.openxmlformats.org/drawingml/2006/chart"

    def walk(node, ancestors):
        tag = local_name(node.tag)
        if tag == "v" and node.text and not is_numberish(node.text):
            if "strCache" in ancestors or "tx" in ancestors:
                supported.append(node.text)
            elif "numCache" in ancestors or "numRef" in ancestors:
                unsupported.append(node.text)
            else:
                unsupported.append(node.text)
        for child in node:
            walk(child, ancestors + [tag])

    walk(root, [])
    return {"supported": supported, "unsupported": unsupported}

def chart_strings(xml_bytes):
    """Visible chart text kept for compatibility with older helper users."""
    classes = chart_text_classes(xml_bytes)
    return classes["supported"]

def is_numberish(s):
    return bool(re.fullmatch(r"[\s0-9.,;:%+\-/()€$£–—]*", s or ""))

def local_name(tag):
    return tag.rsplit("}", 1)[-1] if "}" in tag else tag

def looks_natural_language(s):
    """A run worth translating: has at least one 3+ letter word."""
    return bool(re.search(r"[A-Za-zÀ-ÿ]{3,}", s or ""))

def rel_entries_for(zf, part):
    """Return relationship entries with resolved internal targets."""
    d = os.path.dirname(part)
    base = os.path.basename(part)
    relp = f"{d}/_rels/{base}.rels"
    out = []
    if relp not in zf.namelist():
        return out
    try:
        root = ET.fromstring(zf.read(relp))
    except ET.ParseError:
        return out
    for rel in root.findall("{%s}Relationship" % RELNS):
        tgt = rel.get("Target")
        tid = rel.get("Id")
        typ = rel.get("Type", "")
        if not tgt or rel.get("TargetMode") == "External":
            continue
        if tgt.startswith(".."):
            tgt = os.path.normpath(os.path.join(d, tgt)).replace("\\", "/")
        elif not tgt.startswith("/"):
            tgt = os.path.normpath(os.path.join(d, tgt)).replace("\\", "/")
        else:
            tgt = tgt.lstrip("/")
        out.append({"id": tid, "type": typ, "target": tgt})
    return out

def rels_for(zf, part):
    """Return dict relId->target (resolved part path) for a part's .rels."""
    return {entry["id"]: entry["target"] for entry in rel_entries_for(zf, part)}

def slide_related(zf, slide):
    """Charts plus SmartArt data/drawing parts referenced by a slide."""
    charts, diagram_data, diagram_drawings = [], [], []
    for entry in rel_entries_for(zf, slide):
        tgt = entry["target"]
        if "/charts/chart" in tgt and tgt.endswith(".xml"):
            charts.append(tgt)
        if "/diagrams/" in tgt and tgt.endswith(".xml"):
            if os.path.basename(tgt).startswith("data"):
                diagram_data.append(tgt)
            elif os.path.basename(tgt).startswith("drawing"):
                diagram_drawings.append(tgt)
    diagrams = []
    diagram_drawings = sorted(set(diagram_drawings))
    for data in sorted(set(diagram_data)):
        suffix = re.search(r"data(\d+)\.xml$", os.path.basename(data))
        drawing = None
        if suffix:
            candidate = f"ppt/diagrams/drawing{suffix.group(1)}.xml"
            if candidate in diagram_drawings:
                drawing = candidate
        if drawing is None and len(diagram_drawings) == 1 and len(set(diagram_data)) == 1:
            drawing = diagram_drawings[0]
        diagrams.append({"data": data, "drawing": drawing})
    return sorted(set(charts)), diagrams

def slide_list(zf):
    names = [n for n in zf.namelist() if re.fullmatch(r"ppt/slides/slide\d+\.xml", n)]
    return sorted(names, key=lambda n: int(re.search(r"(\d+)", n).group(1)))

def analyze(orig_path, trans_path):
    zo = zipfile.ZipFile(orig_path)
    zt = zipfile.ZipFile(trans_path)
    slides = slide_list(zo)
    report = []
    for s in slides:
        n = int(re.search(r"(\d+)", s).group(1))
        o_runs = atexts(zo.read(s))
        t_runs = atexts(zt.read(s)) if s in zt.namelist() else []
        o_breaks = count_breaks(zo.read(s))
        t_breaks = count_breaks(zt.read(s)) if s in zt.namelist() else 0
        o_paras = count_paras(zo.read(s))
        t_paras = count_paras(zt.read(s)) if s in zt.namelist() else 0
        # structural run comparison
        o_nl = [r for r in o_runs if looks_natural_language(r)]
        t_nl = [r for r in t_runs if looks_natural_language(r)]
        # untranslated: identical NL runs present in both (multiset intersection)
        from collections import Counter
        o_c, t_c = Counter(o_nl), Counter(t_nl)
        identical_nl = list((o_c & t_c).elements())
        # charts/diagrams
        o_charts, o_diags = slide_related(zo, s)
        ch_total = ch_untrans = ch_unsupported = 0
        dg_total = dg_untrans = dg_mirror_missing = dg_mirror_source = 0
        chart_detail, diag_detail = [], []
        for c in o_charts:
            if c not in zt.namelist():
                continue
            classes_o = chart_text_classes(zo.read(c))
            classes_t = chart_text_classes(zt.read(c))
            cs_o = [x for x in classes_o["supported"] if looks_natural_language(x)]
            cs_t = [x for x in classes_t["supported"] if looks_natural_language(x)]
            unsupported_o = [x for x in classes_o["unsupported"] if looks_natural_language(x)]
            same = list((Counter(cs_o) & Counter(cs_t)).elements())
            ch_total += len(cs_o)
            ch_untrans += len(same)
            ch_unsupported += len(unsupported_o)
            if cs_o:
                chart_detail.append({
                    "part": c,
                    "supported_nl_strings": len(cs_o),
                    "supported_unchanged": len(same),
                    "unsupported_nl_strings": len(unsupported_o),
                })
        for diag in o_diags:
            dpart = diag["data"]
            if dpart not in zt.namelist():
                continue
            ds_o = [x for x in atexts(zo.read(dpart)) if looks_natural_language(x)]
            ds_t = [x for x in atexts(zt.read(dpart)) if looks_natural_language(x)]
            same = list((Counter(ds_o) & Counter(ds_t)).elements())
            dg_total += len(ds_o)
            dg_untrans += len(same)
            mirror_detail = {}
            drawing = diag.get("drawing")
            if drawing and drawing in zo.namelist() and drawing in zt.namelist():
                draw_o = [x for x in atexts(zo.read(drawing)) if looks_natural_language(x)]
                draw_t = [x for x in atexts(zt.read(drawing)) if looks_natural_language(x)]
                missing = list((Counter(ds_t) - Counter(draw_t)).elements())
                source_remaining = list((Counter(draw_o) & Counter(draw_t)).elements())
                dg_mirror_missing += len(missing)
                dg_mirror_source += len(source_remaining)
                mirror_detail = {
                    "drawing_part": drawing,
                    "translated_data_absent_from_drawing": len(missing),
                    "source_language_in_drawing": len(source_remaining),
                    "sample_missing": missing[:6],
                    "sample_source": source_remaining[:6],
                }
            if ds_o:
                item = {"part": dpart, "nl_strings": len(ds_o), "unchanged": len(same)}
                item.update(mirror_detail)
                diag_detail.append(item)

        problems = []
        if len(t_runs) < len(o_runs):
            problems.append(f"LOST_RUNS: orig {len(o_runs)} -> trans {len(t_runs)} (-{len(o_runs)-len(t_runs)})")
        if t_breaks < o_breaks:
            problems.append(f"LOST_LINEBREAKS: orig {o_breaks} -> trans {t_breaks} (-{o_breaks-t_breaks}) [multi-line run/para collapsed]")
        if t_paras < o_paras:
            problems.append(f"LOST_PARAGRAPHS: orig {o_paras} -> trans {t_paras} (-{o_paras-t_paras})")
        if len(o_nl) and len(identical_nl) and len(identical_nl) == len(o_nl):
            problems.append(f"SLIDE_FULLY_UNTRANSLATED: all {len(o_nl)} NL runs identical")
        elif identical_nl:
            problems.append(f"UNTRANSLATED_RUNS: {len(identical_nl)}/{len(o_nl)} NL runs unchanged")
        if ch_total and ch_untrans:
            problems.append(f"SUPPORTED_CHART_TEXT_UNTRANSLATED: {ch_untrans}/{ch_total} supported chart strings unchanged")
        if ch_unsupported:
            problems.append(f"UNSUPPORTED_CHART_TEXT_PRESENT: {ch_unsupported} unsupported chart strings observed")
        if dg_total and dg_untrans:
            problems.append(f"SUPPORTED_DIAGRAM_TEXT_UNTRANSLATED: {dg_untrans}/{dg_total} SmartArt data strings unchanged")
        if dg_mirror_missing or dg_mirror_source:
            problems.append(
                "STALE_SMARTART_DRAWING_MIRROR: "
                f"{dg_mirror_missing} translated data strings absent, "
                f"{dg_mirror_source} source-language drawing strings remain"
            )

        report.append({
            "slide": n,
            "orig_runs": len(o_runs), "trans_runs": len(t_runs),
            "orig_breaks": o_breaks, "trans_breaks": t_breaks,
            "orig_paras": o_paras, "trans_paras": t_paras,
            "orig_nl_runs": len(o_nl), "untranslated_nl_runs": len(identical_nl),
            "charts": {
                "supported_total_nl": ch_total,
                "supported_unchanged": ch_untrans,
                "unsupported_total_nl": ch_unsupported,
                "detail": chart_detail,
            },
            "diagrams": {
                "total_nl": dg_total,
                "unchanged": dg_untrans,
                "stale_mirror_missing": dg_mirror_missing,
                "stale_mirror_source": dg_mirror_source,
                "detail": diag_detail,
            },
            "problems": problems,
            "sample_untranslated": identical_nl[:6],
        })
    return report

def main():
    orig, trans = sys.argv[1], sys.argv[2]
    rep = analyze(orig, trans)
    print(json.dumps({"orig": orig, "trans": trans, "slides": rep}, ensure_ascii=False, indent=1))
    # stderr summary
    name = os.path.basename(trans)
    flagged = [r for r in rep if r["problems"]]
    print(f"\n### {name}: {len(flagged)}/{len(rep)} slides flagged", file=sys.stderr)
    for r in flagged:
        print(f"  slide {r['slide']:>3}: " + " | ".join(r["problems"]), file=sys.stderr)

if __name__ == "__main__":
    main()
