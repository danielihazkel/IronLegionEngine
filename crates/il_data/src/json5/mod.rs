//! JSON5 with source positions (T1-020, REQ-TECH-005, Modding SDK §3.6).

mod parser;
mod value;

pub use parser::{ParseError, parse_json5};
pub use value::{FileId, Key, Num, PathSeg, Span, SpannedValue, ValueKind, span_display};

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> SpannedValue {
        parse_json5(src, FileId(0)).unwrap_or_else(|e| panic!("{e}: {src}"))
    }

    fn err(src: &str) -> ParseError {
        parse_json5(src, FileId(0)).expect_err(src)
    }

    fn at(line: u32, col: u32) -> Span {
        Span {
            file: FileId(0),
            line,
            col,
        }
    }

    #[test]
    fn keys_and_values_carry_positions() {
        let v = parse("{\n  id: \"m:x\",\n  hp: 100,  // comment\n  ranged: { accuracy: 0.6 },\n}");
        assert_eq!(v.span, at(1, 1));
        assert_eq!(v.key_span("id"), Some(at(2, 3)));
        assert_eq!(v.get("id").unwrap().span, at(2, 7));
        assert_eq!(v.key_span("hp"), Some(at(3, 3)));
        assert_eq!(v.get("hp").unwrap().kind, ValueKind::Number(Num::Int(100)));
        let acc = v
            .at_path(&[PathSeg::Key("ranged"), PathSeg::Key("accuracy")])
            .unwrap();
        assert_eq!(acc.span, at(4, 23));
        assert_eq!(acc.kind, ValueKind::Number(Num::Float(0.6)));
    }

    #[test]
    fn json5_syntax_is_accepted() {
        let v = parse(
            r#"/* leading */ {
              unquoted: 'single',
              "quoted": "esc \n \t é \x41 \" \' \\ \/",
              trailing: [1, 2, 3,],
              hex: 0x1F, negHex: -0x10, plus: +5, leadDot: .5, trailDot: 5., exp: 1e3, expNeg: -2.5E-2,
              cont: "line \
continued",
              $dollar_: true, _under: null,
              nested: { a: { b: [ { c: false } ] } },
            } // trailing comment"#,
        );
        assert_eq!(v.get("unquoted").unwrap().as_str(), Some("single"));
        assert_eq!(
            v.get("quoted").unwrap().as_str(),
            Some("esc \n \t é A \" ' \\ /")
        );
        assert_eq!(v.get("trailing").unwrap().as_array().unwrap().len(), 3);
        assert_eq!(v.get("hex").unwrap().kind, ValueKind::Number(Num::Int(31)));
        assert_eq!(
            v.get("negHex").unwrap().kind,
            ValueKind::Number(Num::Int(-16))
        );
        assert_eq!(v.get("plus").unwrap().kind, ValueKind::Number(Num::Int(5)));
        assert_eq!(
            v.get("leadDot").unwrap().kind,
            ValueKind::Number(Num::Float(0.5))
        );
        assert_eq!(
            v.get("trailDot").unwrap().kind,
            ValueKind::Number(Num::Float(5.0))
        );
        assert_eq!(
            v.get("exp").unwrap().kind,
            ValueKind::Number(Num::Float(1000.0))
        );
        assert_eq!(
            v.get("expNeg").unwrap().kind,
            ValueKind::Number(Num::Float(-0.025))
        );
        assert_eq!(v.get("cont").unwrap().as_str(), Some("line continued"));
        assert_eq!(v.get("$dollar_").unwrap().as_bool(), Some(true));
        assert_eq!(v.get("_under").unwrap().kind, ValueKind::Null);
        let c = v
            .at_path(&[
                PathSeg::Key("nested"),
                PathSeg::Key("a"),
                PathSeg::Key("b"),
                PathSeg::Index(0),
                PathSeg::Key("c"),
            ])
            .unwrap();
        assert_eq!(c.as_bool(), Some(false));
    }

    #[test]
    fn to_json_matches_serde_json_for_plain_documents() {
        let src = r#"{ "a": [1, 2.5, "x", true, null], "b": { "c": -3 } }"#;
        let ours = parse(src).to_json();
        let theirs: serde_json::Value = serde_json::from_str(src).unwrap();
        assert_eq!(ours, theirs);
        let big = parse("99999999999999999999");
        assert!(matches!(big.kind, ValueKind::Number(Num::Float(_))));
    }

    #[test]
    fn surrogate_pairs_and_crlf_lines() {
        let v = parse("{\r\n  emoji: \"\\uD83D\\uDE00\",\r\n  n: 1\r\n}");
        assert_eq!(v.get("emoji").unwrap().as_str(), Some("😀"));
        assert_eq!(v.key_span("n"), Some(at(3, 3)));
    }

    #[test]
    fn errors_carry_the_right_position() {
        assert_eq!(err("{ a: 1 b: 2 }").span, at(1, 8));
        assert_eq!(err("{\n  a: 1,\n  a: 2\n}").span, at(3, 3));
        assert!(
            err("{\n  a: 1,\n  a: 2\n}")
                .message
                .contains("duplicate key")
        );
        assert_eq!(err("[1, 2").span, at(1, 6));
        assert_eq!(err("{ a: Infinity }").span, at(1, 6));
        assert_eq!(err("{ a: -Infinity }").span, at(1, 6));
        assert_eq!(err("{ a: NaN }").span, at(1, 6));
        assert_eq!(err("{ a: 1e999 }").span, at(1, 6));
        assert_eq!(err("\"open").span, at(1, 1));
        assert_eq!(err("{ a: \"x\ny\" }").span, at(1, 8));
        assert_eq!(err("/* never closed").span, at(1, 1));
        assert_eq!(err("{} extra").span, at(1, 4));
        assert_eq!(err("{ a: 0x }").span, at(1, 6));
        assert_eq!(err("{ a: 1.2.3 }").span, at(1, 9));
        assert!(err("").message.contains("end of file"));
    }

    #[test]
    fn document_may_be_any_value() {
        assert_eq!(parse("42").kind, ValueKind::Number(Num::Int(42)));
        assert_eq!(parse("  'hi' ").as_str(), Some("hi"));
        assert_eq!(parse("[]").as_array().unwrap().len(), 0);
        assert_eq!(parse("\u{FEFF}{}").as_object().unwrap().len(), 0);
    }

    #[test]
    fn remove_and_get_mut_edit_objects() {
        let mut v = parse("{ a: 1, b: 2 }");
        let (k, removed) = v.remove("a").unwrap();
        assert_eq!(k.name, "a");
        assert_eq!(removed.kind, ValueKind::Number(Num::Int(1)));
        assert!(v.get("a").is_none());
        v.get_mut("b").unwrap().kind = ValueKind::Bool(true);
        assert_eq!(v.get("b").unwrap().as_bool(), Some(true));
        assert_eq!(v.type_name(), "an object");
    }
}
