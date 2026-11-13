//! Admission check for sticker SVG bytes: deny by default, so a file with scripts, external
//! references, a DTD or any element outside the drawing subset never reaches the cache.
use engine::StickerError;
use roxmltree::{Document, Node, NodeType, ParsingOptions};

/// Largest SVG a sticker may be.
pub const MAX_SVG_BYTES: usize = 64 * 1024;
/// Most XML nodes one SVG may hold.
const MAX_NODES: u32 = 20_000;
/// Deepest element nesting accepted.
const MAX_DEPTH: usize = 64;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
/// The drawing subset: shapes, groups, gradients, clipping and in-document `use`.
const ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "defs",
    "path",
    "circle",
    "ellipse",
    "rect",
    "line",
    "polyline",
    "polygon",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "use",
    "title",
    "desc",
];
/// Attribute value fragments that carry script or inline data.
const FORBIDDEN_VALUES: &[&str] = &["javascript:", "@import", "expression(", "data:"];

/// Admits an SVG sticker, or refuses it with the first reason found. Nothing is rewritten: bytes
/// are kept exactly as pinned, so the hash stays meaningful.
///
/// Refused: more than [`MAX_SVG_BYTES`], invalid UTF-8, malformed XML, any DTD (entity expansion),
/// a processing instruction, a root that is not `svg`, an element outside the drawing subset or its
/// namespace, an `on*` event attribute, an `href` that is not a `#fragment`, a `url(...)` that is
/// not a `#fragment`, and script, `@import`, `expression(` or `data:` inside any attribute value.
///
/// # Errors
///
/// [`StickerError::UnsafeSvg`] naming the rule.
pub fn check_svg(bytes: &[u8]) -> Result<(), StickerError> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err(StickerError::UnsafeSvg("larger than the sticker limit"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| StickerError::UnsafeSvg("not UTF-8 text"))?;
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_NODES,
    };
    let document = Document::parse_with_options(text, options)
        .map_err(|_| StickerError::UnsafeSvg("not well-formed XML without a DTD"))?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NAMESPACE) {
        return Err(StickerError::UnsafeSvg("root is not an svg element"));
    }
    for node in document.root().descendants() {
        match node.node_type() {
            NodeType::PI => return Err(StickerError::UnsafeSvg("processing instruction")),
            NodeType::Element => check_element(&node)?,
            _ => {}
        }
    }
    Ok(())
}

fn check_element(node: &Node<'_, '_>) -> Result<(), StickerError> {
    if node.ancestors().count() > MAX_DEPTH {
        return Err(StickerError::UnsafeSvg("nested too deeply"));
    }
    let name = node.tag_name();
    if name.namespace() != Some(SVG_NAMESPACE) || !ELEMENTS.contains(&name.name()) {
        return Err(StickerError::UnsafeSvg(
            "element outside the drawing subset",
        ));
    }
    for attribute in node.attributes() {
        let local = attribute.name().to_ascii_lowercase();
        let value = attribute.value().to_ascii_lowercase();
        if local.starts_with("on") {
            return Err(StickerError::UnsafeSvg("event handler attribute"));
        }
        if local == "href" && !value.trim_start().starts_with('#') {
            return Err(StickerError::UnsafeSvg("reference outside the document"));
        }
        if FORBIDDEN_VALUES.iter().any(|bad| value.contains(bad)) {
            return Err(StickerError::UnsafeSvg("script or inline data in a value"));
        }
        if value.split("url(").skip(1).any(|rest| {
            !rest
                .trim_start()
                .trim_start_matches(['\'', '"'])
                .starts_with('#')
        }) {
            return Err(StickerError::UnsafeSvg("url() outside the document"));
        }
    }
    Ok(())
}
