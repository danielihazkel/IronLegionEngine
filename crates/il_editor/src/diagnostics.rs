//! The document's diagnostics (T3-063, TDD §16): the map schema run on the
//! text Save would write, the references the registries must resolve, and
//! the warnings a map can carry and still play. An error blocks Save; the
//! panel lists each with its field, and a click shows the feature.

use std::path::Path;

use glam::Vec2;
use il_core::Scalar;
use il_data::json5::{FileId, parse_json5};
use il_data::{Diagnostic, Diagnostics, KindTag, MapEdge, Registries, Severity, validate_value};

use crate::document::MapDocument;
use crate::vector::to_vec2;

/// The display file name diagnostics carry.
pub const DOCUMENT_FILE: &str = "<editor>";
/// A reinforcement edge's side zone must come this close to that edge
/// (the `genmap` band margin).
pub const EDGE_REACH_M: f32 = 40.0;

fn error(field: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::file_level(DOCUMENT_FILE, message).field(field)
}

fn warning(field: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    error(field, message).warning()
}

/// Every problem with `doc` as Save would write it, errors first.
/// `target_namespaces` are the target mod's namespaces when known.
pub fn check(
    doc: &MapDocument,
    regs: &Registries,
    target_namespaces: Option<&[String]>,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut baked = doc.clone();
    if let Err(e) = baked.bake() {
        out.push(error("zones", e.to_string()));
    }
    let d = &baked.def;

    // The schema, on the exact text Save writes.
    let text = baked.text();
    match parse_json5(&text, FileId(0)) {
        Ok(value) => {
            let mut diags = Diagnostics::new();
            validate_value(KindTag::Map, &value, Path::new(DOCUMENT_FILE), &mut diags);
            out.extend(diags.0);
        }
        Err(e) => out.push(error("", e.to_string())),
    }

    // References.
    if regs.zones.lookup(&d.base_zone).is_none() {
        out.push(error(
            "base_zone",
            format!("zone type {} does not exist", d.base_zone),
        ));
    }
    for (i, z) in d.zones.iter().enumerate() {
        if regs.zones.lookup(&z.type_id).is_none() {
            out.push(error(
                format!("zones[{i}].type"),
                format!("zone type {} does not exist", z.type_id),
            ));
        }
    }

    // Warnings: the map plays, but probably not as meant.
    let (w, h) = (d.size.w.to_f32_render(), d.size.h.to_f32_render());
    for (i, e) in d.reinforcement_edges.iter().enumerate() {
        let field = format!("reinforcement_edges[{i}]");
        let Some(zone) = d.deployment.iter().find(|z| z.side == e.side) else {
            out.push(warning(
                field,
                format!("side {} has no deployment polygon", e.side),
            ));
            continue;
        };
        let pts: Vec<Vec2> = zone.polygon.iter().map(|p| to_vec2(*p)).collect();
        let reach = match e.edge {
            MapEdge::South => pts.iter().map(|p| p.y).fold(f32::MAX, f32::min),
            MapEdge::North => pts.iter().map(|p| h - p.y).fold(f32::MAX, f32::min),
            MapEdge::West => pts.iter().map(|p| p.x).fold(f32::MAX, f32::min),
            MapEdge::East => pts.iter().map(|p| w - p.x).fold(f32::MAX, f32::min),
        };
        if reach > EDGE_REACH_M {
            out.push(warning(
                field,
                format!(
                    "side {}'s deployment zone stays {reach:.0} m from its {:?} edge",
                    e.side, e.edge
                ),
            ));
        }
    }
    if let Some(ns) = target_namespaces {
        let own = d.id.as_str().split_once(':').map(|(n, _)| n.to_string());
        if own.as_ref().is_some_and(|n| !ns.contains(n)) && regs.maps.lookup(&d.id).is_none() {
            out.push(warning(
                "id",
                format!(
                    "{} is outside the target mod's namespaces ({})",
                    d.id,
                    ns.join(", ")
                ),
            ));
        }
    }
    for (i, r) in d.rivers.iter().enumerate() {
        let line: Vec<Vec2> = r.points.iter().map(|p| to_vec2(*p)).collect();
        let crossed = d.zones.iter().any(|z| {
            regs.zones
                .lookup(&z.type_id)
                .is_some_and(|hd| regs.zones.get(hd).crossing)
                && touches(
                    &z.polygon.iter().map(|p| to_vec2(*p)).collect::<Vec<_>>(),
                    &line,
                )
        });
        if !crossed {
            out.push(warning(
                format!("rivers[{i}]"),
                "no ford or bridge crosses this river",
            ));
        }
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.severity));
    out
}

/// Whether an error stands among `diags`.
pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

/// Whether polygon `poly` and polyline `line` meet: an edge crossing, or a
/// line point inside the polygon.
fn touches(poly: &[Vec2], line: &[Vec2]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let inside = |p: Vec2| {
        let mut c = false;
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (poly[i], poly[j]);
            if (a.y <= p.y) != (b.y <= p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
                c = !c;
            }
            j = i;
        }
        c
    };
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let meets = |p1: Vec2, p2: Vec2, q1: Vec2, q2: Vec2| {
        let (d1, d2) = (cross(q1, q2, p1), cross(q1, q2, p2));
        let (d3, d4) = (cross(p1, p2, q1), cross(p1, p2, q2));
        d1 * d2 <= 0.0 && d3 * d4 <= 0.0
    };
    if line.iter().any(|p| inside(*p)) {
        return true;
    }
    line.windows(2)
        .any(|s| (0..n).any(|k| meets(s[0], s[1], poly[k], poly[(k + 1) % n])))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use il_core::V2;
    use il_data::{ContentId, ReinforcementEdge};

    use super::*;

    fn regs() -> Registries {
        let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        il_data::load_roots(&[root]).unwrap_or_else(|d| panic!("{d}"))
    }

    fn test_field(regs: &Registries) -> MapDocument {
        let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        MapDocument::from_registry(regs, &ContentId::new("rome:test_field").unwrap(), &[root])
            .unwrap()
    }

    fn errors(d: &[Diagnostic]) -> Vec<&str> {
        d.iter()
            .filter(|x| x.severity == Severity::Error)
            .map(|x| x.field.as_str())
            .collect()
    }

    fn warnings(d: &[Diagnostic]) -> Vec<&str> {
        d.iter()
            .filter(|x| x.severity == Severity::Warning)
            .map(|x| x.field.as_str())
            .collect()
    }

    #[test]
    fn the_test_field_is_clean() {
        let regs = regs();
        let d = check(&test_field(&regs), &regs, Some(&["rome".to_string()]));
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn each_error_names_its_field() {
        let regs = regs();
        let mut doc = test_field(&regs);
        doc.def.zones[1].polygon.truncate(2);
        let d = check(&doc, &regs, None);
        assert_eq!(errors(&d).len(), 1, "{d:?}");
        assert!(errors(&d)[0].starts_with("zones[1].polygon"), "{d:?}");

        let mut doc = test_field(&regs);
        doc.def.zones[0].type_id = ContentId::new("rome:lava").unwrap();
        assert_eq!(errors(&check(&doc, &regs, None)), ["zones[0].type"]);

        let mut doc = test_field(&regs);
        doc.def.deployment.clear();
        doc.def.reinforcement_edges.clear();
        let d = check(&doc, &regs, None);
        assert_eq!(errors(&d), ["deployment"], "{d:?}");
        assert!(has_errors(&d));
    }

    #[test]
    fn each_warning_triggers() {
        let regs = regs();
        // An edge whose side has no zone.
        let mut doc = test_field(&regs);
        doc.def.reinforcement_edges.push(ReinforcementEdge {
            side: 5,
            edge: MapEdge::West,
        });
        let n = doc.def.reinforcement_edges.len() - 1;
        assert_eq!(
            warnings(&check(&doc, &regs, None)),
            [format!("reinforcement_edges[{n}]")]
        );
        // An edge its side's zone does not reach.
        let mut doc = test_field(&regs);
        let side = doc.def.deployment[0].side;
        let far = if doc
            .def
            .reinforcement_edges
            .iter()
            .any(|e| e.side == side && e.edge == MapEdge::East)
        {
            MapEdge::West
        } else {
            MapEdge::East
        };
        doc.def.deployment[0].polygon = [(300.0, 200.0), (340.0, 200.0), (340.0, 240.0)]
            .iter()
            .map(|(x, y)| V2::from_f32_data(*x, *y))
            .collect();
        doc.def.reinforcement_edges = vec![ReinforcementEdge { side, edge: far }];
        assert_eq!(
            warnings(&check(&doc, &regs, None)),
            ["reinforcement_edges[0]"]
        );
        // An id outside the target's namespaces.
        let mut doc = test_field(&regs);
        doc.def.id = ContentId::new("rome:elsewhere").unwrap();
        assert_eq!(
            warnings(&check(&doc, &regs, Some(&["mymod".to_string()]))),
            ["id"]
        );
        // A river with no crossing.
        let mut doc = test_field(&regs);
        doc.def.zones.retain(|z| {
            !z.type_id.as_str().ends_with(":bridge") && !z.type_id.as_str().ends_with(":ford")
        });
        assert_eq!(warnings(&check(&doc, &regs, None)), ["rivers[0]"]);
    }
}
