# 030. PPTX Presentation Model

The PPTX domain model sits above OPC and XML. It provides semantic access without requiring agents to understand raw OpenXML.

## Core Types

```rust
struct PresentationDocument {
    package: Package,
    presentation: Presentation,
}

struct Presentation {
    part_name: PartName,
    slides: Vec<SlideRef>,
    metadata: PresentationMetadata,
}

struct Slide {
    id: SlideId,
    part_name: PartName,
    rels_part_name: PartName,
    layout: Option<SlideLayoutRef>,
    elements: Vec<SlideElement>,
}
```

## Slide Elements

```rust
enum SlideElement {
    Shape(Shape),
    TextBox(TextBox),
    Picture(Picture),
    GraphicFrame(GraphicFrame),
    Group(GroupShape),
    Unknown(UnknownElement),
}
```

Every element exposed to agents should include:

- Stable agent-facing ID.
- Source slide ID.
- Source part path.
- Backing XML location or element key.
- Element kind.
- Editable fields.

## Preservation

- Unsupported elements must remain in the slide XML.
- Unknown element order must be preserved.
- Shape tree order must be preserved because it affects z-order.
- Typed edits must only touch the target element and required package graph dependencies.

## Units

Native persisted units are EMUs. Public helper APIs may expose inches, points, or pixels, but the canonical model should preserve EMU values.

## Discovery

The presentation model is hydrated from OPC relationships:

- The presentation part is located from the root Office document relationship.
- Slide order comes from the presentation part's slide ID list, not ZIP entry order.
- Slide parts are resolved through presentation relationships.
- Layout, master, theme, media, and chart references are resolved through each source part's relationship set.

Implementations may expose common paths such as `ppt/presentation.xml` in JSON examples, but must not rely on those paths as the only valid PPTX layout.
