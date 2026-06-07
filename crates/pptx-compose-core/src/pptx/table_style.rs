use std::collections::BTreeMap;

use crate::{
    opc::{package::Package, part_name::PartName},
    xml::{
        document::{XmlElement, XmlNode},
        parser::parse_document,
    },
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TableStyleCatalog {
    pub default_style_id: Option<String>,
    styles: BTreeMap<String, TableStyle>,
}

impl TableStyleCatalog {
    #[must_use]
    pub fn from_root(root: &XmlElement) -> Self {
        let default_style_id = attr(root, "def").map(str::to_owned);
        let styles = child_elements(root, "tblStyle")
            .filter_map(TableStyle::from_element)
            .map(|style| (style.style_id.clone(), style))
            .collect();

        Self {
            default_style_id,
            styles,
        }
    }

    #[must_use]
    pub fn from_package(package: &Package) -> Option<Self> {
        let part_name = PartName::from_zip_entry("ppt/tableStyles.xml").ok()?;
        let part = package.parts().get(&part_name)?;
        let document = parse_document(part.bytes()).ok()?;
        let root = document.root_element()?;
        Some(Self::from_root(root))
    }

    #[must_use]
    pub fn style(&self, style_id: &str) -> Option<&TableStyle> {
        self.styles.get(style_id)
    }

    #[must_use]
    pub fn resolve_cell_defaults(
        &self,
        table_properties: &TableProperties,
        row: usize,
        col: usize,
        row_count: usize,
        col_count: usize,
    ) -> Option<TableCellTextDefaults> {
        let style_id = table_properties
            .style_id
            .as_deref()
            .or(self.default_style_id.as_deref())?;
        self.style(style_id).map(|style| {
            style.resolve_cell_defaults(style_id, table_properties, row, col, row_count, col_count)
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableStyle {
    pub style_id: String,
    pub style_name: Option<String>,
    regions: BTreeMap<TableStyleRegion, TableStyleRegionDefaults>,
}

impl TableStyle {
    fn from_element(element: &XmlElement) -> Option<Self> {
        let style_id = attr(element, "styleId")?.to_owned();
        let style_name = attr(element, "styleName").map(str::to_owned);
        let mut regions = BTreeMap::new();

        for child in child_elements_any(element) {
            let Some(region) = TableStyleRegion::from_local_name(&child.name.local_name) else {
                continue;
            };
            regions.insert(region, TableStyleRegionDefaults::from_element(child));
        }

        Some(Self {
            style_id,
            style_name,
            regions,
        })
    }

    fn resolve_cell_defaults(
        &self,
        style_id: &str,
        table_properties: &TableProperties,
        row: usize,
        col: usize,
        row_count: usize,
        col_count: usize,
    ) -> TableCellTextDefaults {
        let applied_regions = applied_regions(table_properties, row, col, row_count, col_count);
        let mut defaults = TableCellTextDefaults {
            style_id: style_id.to_owned(),
            applied_regions: Vec::new(),
            paragraph_properties: None,
            run_properties: TableRunProperties::default(),
        };

        for region in applied_regions {
            let Some(region_defaults) = self.regions.get(&region) else {
                continue;
            };
            defaults.applied_regions.push(region);
            if let Some(paragraph_properties) = &region_defaults.paragraph_properties {
                defaults.paragraph_properties = Some(paragraph_properties.clone());
            }
            defaults
                .run_properties
                .merge_from(&region_defaults.run_properties);
        }

        defaults
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TableProperties {
    pub style_id: Option<String>,
    pub first_row: bool,
    pub first_col: bool,
    pub last_row: bool,
    pub last_col: bool,
    pub band_row: bool,
    pub band_col: bool,
}

impl TableProperties {
    #[must_use]
    pub fn from_tbl_pr(tbl_pr: &XmlElement) -> Self {
        Self {
            style_id: child_text(tbl_pr, "tableStyleId").filter(|value| !value.is_empty()),
            first_row: bool_attr(tbl_pr, "firstRow"),
            first_col: bool_attr(tbl_pr, "firstCol"),
            last_row: bool_attr(tbl_pr, "lastRow"),
            last_col: bool_attr(tbl_pr, "lastCol"),
            band_row: bool_attr(tbl_pr, "bandRow"),
            band_col: bool_attr(tbl_pr, "bandCol"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableCellTextDefaults {
    pub style_id: String,
    pub applied_regions: Vec<TableStyleRegion>,
    pub paragraph_properties: Option<XmlElement>,
    pub run_properties: TableRunProperties,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TableStyleRegion {
    WholeTable,
    Band1Horizontal,
    Band2Horizontal,
    Band1Vertical,
    Band2Vertical,
    FirstColumn,
    LastColumn,
    FirstRow,
    LastRow,
    NorthwestCell,
    NortheastCell,
    SouthwestCell,
    SoutheastCell,
}

impl TableStyleRegion {
    fn from_local_name(local_name: &str) -> Option<Self> {
        Some(match local_name {
            "wholeTbl" => Self::WholeTable,
            "band1H" => Self::Band1Horizontal,
            "band2H" => Self::Band2Horizontal,
            "band1V" => Self::Band1Vertical,
            "band2V" => Self::Band2Vertical,
            "firstCol" => Self::FirstColumn,
            "lastCol" => Self::LastColumn,
            "firstRow" => Self::FirstRow,
            "lastRow" => Self::LastRow,
            "nwCell" => Self::NorthwestCell,
            "neCell" => Self::NortheastCell,
            "swCell" => Self::SouthwestCell,
            "seCell" => Self::SoutheastCell,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TableStyleRegionDefaults {
    paragraph_properties: Option<XmlElement>,
    run_properties: TableRunProperties,
}

impl TableStyleRegionDefaults {
    fn from_element(region: &XmlElement) -> Self {
        let text_style = child_element(region, "tcTxStyle");
        Self {
            paragraph_properties: text_style
                .and_then(|style| child_element(style, "pPr"))
                .cloned(),
            run_properties: text_style.map_or_else(TableRunProperties::default, run_properties),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TableRunProperties {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub font_ref_idx: Option<String>,
    pub color: Option<TableColor>,
}

impl TableRunProperties {
    fn merge_from(&mut self, other: &Self) {
        if other.bold.is_some() {
            self.bold = other.bold;
        }
        if other.italic.is_some() {
            self.italic = other.italic;
        }
        if other.font_ref_idx.is_some() {
            self.font_ref_idx.clone_from(&other.font_ref_idx);
        }
        if other.color.is_some() {
            self.color.clone_from(&other.color);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableColor {
    pub kind: String,
    pub value: Option<String>,
}

fn applied_regions(
    table_properties: &TableProperties,
    row: usize,
    col: usize,
    row_count: usize,
    col_count: usize,
) -> Vec<TableStyleRegion> {
    let mut regions = vec![TableStyleRegion::WholeTable];
    if row_count == 0 || col_count == 0 || row >= row_count || col >= col_count {
        return regions;
    }

    let is_first_row = table_properties.first_row && row == 0;
    let is_last_row = table_properties.last_row && row + 1 == row_count;
    let is_first_col = table_properties.first_col && col == 0;
    let is_last_col = table_properties.last_col && col + 1 == col_count;

    if table_properties.band_row
        && !is_first_row
        && !is_last_row
        && let Some(region) = band_region(
            row,
            usize::from(table_properties.first_row),
            TableStyleRegion::Band1Horizontal,
            TableStyleRegion::Band2Horizontal,
        )
    {
        regions.push(region);
    }
    if table_properties.band_col
        && !is_first_col
        && !is_last_col
        && let Some(region) = band_region(
            col,
            usize::from(table_properties.first_col),
            TableStyleRegion::Band1Vertical,
            TableStyleRegion::Band2Vertical,
        )
    {
        regions.push(region);
    }
    if is_first_col {
        regions.push(TableStyleRegion::FirstColumn);
    }
    if is_last_col {
        regions.push(TableStyleRegion::LastColumn);
    }
    if is_first_row {
        regions.push(TableStyleRegion::FirstRow);
    }
    if is_last_row {
        regions.push(TableStyleRegion::LastRow);
    }
    if is_first_row && is_first_col {
        regions.push(TableStyleRegion::NorthwestCell);
    }
    if is_first_row && is_last_col {
        regions.push(TableStyleRegion::NortheastCell);
    }
    if is_last_row && is_first_col {
        regions.push(TableStyleRegion::SouthwestCell);
    }
    if is_last_row && is_last_col {
        regions.push(TableStyleRegion::SoutheastCell);
    }

    regions
}

fn band_region(
    index: usize,
    offset: usize,
    odd: TableStyleRegion,
    even: TableStyleRegion,
) -> Option<TableStyleRegion> {
    let body_index = index.checked_sub(offset)?;
    Some(if body_index % 2 == 0 { odd } else { even })
}

fn run_properties(tc_tx_style: &XmlElement) -> TableRunProperties {
    TableRunProperties {
        bold: attr(tc_tx_style, "b").and_then(on_off),
        italic: attr(tc_tx_style, "i").and_then(on_off),
        font_ref_idx: child_element(tc_tx_style, "fontRef")
            .and_then(|font_ref| attr(font_ref, "idx"))
            .map(str::to_owned),
        color: color_child(tc_tx_style),
    }
}

fn color_child(element: &XmlElement) -> Option<TableColor> {
    child_elements_any(element)
        .find(|child| child.name.local_name.ends_with("Clr"))
        .map(|color| TableColor {
            kind: color.name.local_name.clone(),
            value: attr(color, "val").map(str::to_owned),
        })
}

fn child_element<'a>(element: &'a XmlElement, local_name: &'a str) -> Option<&'a XmlElement> {
    child_elements(element, local_name).next()
}

fn child_elements<'a>(
    element: &'a XmlElement,
    local_name: &'a str,
) -> impl Iterator<Item = &'a XmlElement> + 'a {
    child_elements_any(element).filter(move |child| child.name.local_name == local_name)
}

fn child_elements_any(element: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    element.children.iter().filter_map(XmlNode::as_element)
}

fn child_text(element: &XmlElement, local_name: &str) -> Option<String> {
    let child = child_element(element, local_name)?;
    let mut text = String::new();
    collect_text(child, &mut text);
    Some(text)
}

fn collect_text(element: &XmlElement, text: &mut String) {
    for child in &element.children {
        match child {
            XmlNode::Text(value) | XmlNode::CData(value) => text.push_str(value),
            XmlNode::Element(element) => collect_text(element, text),
            XmlNode::Comment(_)
            | XmlNode::ProcessingInstruction(_)
            | XmlNode::DocType(_)
            | XmlNode::GeneralRef(_) => {}
        }
    }
}

fn attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}

fn bool_attr(element: &XmlElement, local_name: &str) -> bool {
    attr(element, local_name).is_some_and(|value| on_off(value) == Some(true))
}

fn on_off(value: &str) -> Option<bool> {
    match value {
        "1" | "true" | "True" | "TRUE" | "on" => Some(true),
        "0" | "false" | "False" | "FALSE" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TableColor, TableProperties, TableRunProperties, TableStyleCatalog, TableStyleRegion,
    };
    use crate::xml::parser::parse_document;

    #[test]
    fn resolves_header_body_and_banded_table_defaults() {
        let catalog_document = parse_document(table_styles_xml()).expect("table styles parse");
        let catalog_root = catalog_document
            .root_element()
            .expect("table styles root exists");
        let catalog = TableStyleCatalog::from_root(catalog_root);

        let tbl_pr_document = parse_document(tbl_pr_xml()).expect("tblPr parses");
        let tbl_pr = tbl_pr_document.root_element().expect("tblPr root exists");
        let table_properties = TableProperties::from_tbl_pr(tbl_pr);

        let header = catalog
            .resolve_cell_defaults(&table_properties, 0, 0, 4, 3)
            .expect("style resolves");
        assert_eq!(
            header.applied_regions,
            vec![
                TableStyleRegion::WholeTable,
                TableStyleRegion::FirstColumn,
                TableStyleRegion::FirstRow,
                TableStyleRegion::NorthwestCell,
            ]
        );
        assert_eq!(
            header.run_properties,
            TableRunProperties {
                bold: Some(true),
                italic: Some(true),
                font_ref_idx: Some("major".to_owned()),
                color: Some(TableColor {
                    kind: "schemeClr".to_owned(),
                    value: Some("accent2".to_owned()),
                }),
            }
        );
        assert_eq!(
            header
                .paragraph_properties
                .as_ref()
                .and_then(|p_pr| p_pr.attributes.first())
                .map(|attribute| attribute.value.as_str()),
            Some("ctr")
        );

        let body_band_1 = catalog
            .resolve_cell_defaults(&table_properties, 1, 1, 4, 3)
            .expect("style resolves");
        assert_eq!(
            body_band_1.applied_regions,
            vec![
                TableStyleRegion::WholeTable,
                TableStyleRegion::Band1Horizontal,
            ]
        );
        assert_eq!(body_band_1.run_properties.bold, Some(false));
        assert_eq!(
            body_band_1.run_properties.color,
            Some(TableColor {
                kind: "schemeClr".to_owned(),
                value: Some("dk1".to_owned()),
            })
        );

        let body_band_2 = catalog
            .resolve_cell_defaults(&table_properties, 2, 1, 4, 3)
            .expect("style resolves");
        assert_eq!(
            body_band_2.applied_regions,
            vec![
                TableStyleRegion::WholeTable,
                TableStyleRegion::Band2Horizontal,
            ]
        );
        assert_eq!(body_band_2.run_properties.bold, Some(false));
        assert_eq!(
            body_band_2.run_properties.color,
            Some(TableColor {
                kind: "schemeClr".to_owned(),
                value: Some("lt1".to_owned()),
            })
        );
    }

    #[test]
    fn falls_back_to_catalog_default_style() {
        let catalog_document = parse_document(table_styles_xml()).expect("table styles parse");
        let catalog = TableStyleCatalog::from_root(
            catalog_document
                .root_element()
                .expect("table styles root exists"),
        );
        let tbl_pr_document = parse_document(
            br#"<a:tblPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"/>"#,
        )
        .expect("tblPr parses");
        let table_properties = TableProperties::from_tbl_pr(
            tbl_pr_document.root_element().expect("tblPr root exists"),
        );

        let defaults = catalog
            .resolve_cell_defaults(&table_properties, 1, 0, 2, 2)
            .expect("default style resolves");

        assert_eq!(defaults.style_id, "{STYLE-A}");
        assert_eq!(defaults.applied_regions, vec![TableStyleRegion::WholeTable]);
    }

    fn table_styles_xml() -> &'static [u8] {
        br#"
<a:tblStyleLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" def="{STYLE-A}">
  <a:tblStyle styleId="{STYLE-A}" styleName="Synthetic Style">
    <a:wholeTbl>
      <a:tcTxStyle b="off">
        <a:pPr algn="ctr"/>
        <a:fontRef idx="minor"/>
        <a:schemeClr val="dk1"/>
      </a:tcTxStyle>
    </a:wholeTbl>
    <a:band1H>
      <a:tcTxStyle>
        <a:schemeClr val="dk1"/>
      </a:tcTxStyle>
    </a:band1H>
    <a:band2H>
      <a:tcTxStyle>
        <a:schemeClr val="lt1"/>
      </a:tcTxStyle>
    </a:band2H>
    <a:firstCol>
      <a:tcTxStyle i="on"/>
    </a:firstCol>
    <a:firstRow>
      <a:tcTxStyle b="on">
        <a:fontRef idx="major"/>
      </a:tcTxStyle>
    </a:firstRow>
    <a:nwCell>
      <a:tcTxStyle>
        <a:schemeClr val="accent2"/>
      </a:tcTxStyle>
    </a:nwCell>
  </a:tblStyle>
</a:tblStyleLst>
"#
    }

    fn tbl_pr_xml() -> &'static [u8] {
        br#"
<a:tblPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
         firstRow="1" firstCol="1" bandRow="1">
  <a:tableStyleId>{STYLE-A}</a:tableStyleId>
</a:tblPr>
"#
    }
}
