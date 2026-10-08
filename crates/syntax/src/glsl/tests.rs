use super::{Rejection, check};

const SEED: &str = "
attribute vec3 position;
uniform   mat4 view;
varying   vec2 vcoord;

void main () {
  gl_Position = view * vec4(position, 1.0);
  vcoord = position.xy;
}
";

fn offset(text: &str) -> Option<usize> {
    match check(text) {
        Err(Rejection::At { offset, .. }) => Some(offset),
        _ => None,
    }
}

fn body(statements: &str) -> String {
    format!("void main () {{ {statements} }}")
}

#[test]
fn shaders_parse() {
    assert_eq!(check(SEED), Ok(()));
    assert_eq!(check("precision mediump float; // done"), Ok(()));
    for statements in [
        "x += 1; y = x < 2 ? a.b[0] : -f(void);",
        "for (int i = 0; i < 3; i++) { if (i == 1) continue; else break; }",
        "float f = 1e5 f + 0x1F + 017u + .5 + 2.;",
        "struct S { float a, b[2]; } s; return;",
        "/* a\n comment */ discard;",
    ] {
        assert_eq!(check(&body(statements)), Ok(()), "{statements}");
    }
}

#[test]
fn shaders_need_a_declaration() {
    assert_eq!(offset(""), Some(0));
    assert_eq!(offset("  // nothing"), Some(12));
}

#[test]
fn quirks_follow_upstream() {
    // `<` takes the first character of `<<=`.
    assert!(check(&body("x <<= 2;")).is_err());
    // A leading zero before a non-octal digit.
    assert!(check(&body("float f = 0e5;")).is_err());
    assert!(check(&body("float f = 08.5;")).is_err());
    // A parameter's qualifier needs no boundary: `input`, a reserved
    // word, reads as `in` and a type `put`.
    assert!(check("void f(input x) {}").is_ok());
}

#[test]
fn rejections_point_where_parsing_stopped() {
    assert_eq!(offset("void main () { x = ; }"), Some(19));
    assert_eq!(offset("void main () { /* open }"), Some(24));
    assert_eq!(offset("int x = 1;\nint __y;"), Some(18));
}

#[test]
fn deep_nesting_is_rejected() {
    let deep = format!("int x = {}1{};", "(".repeat(100_000), ")".repeat(100_000));
    assert!(matches!(check(&deep), Err(Rejection::TooDeep(_))));
}

#[test]
fn rejections_carry_parsecs_message() {
    // As elm 0.19.1, 0.19.2 and 0.19.3 quote it for the seed's
    // `vcoord = co]ord.xy;`.
    let Err(Rejection::At { message, .. }) = check(&body("vcoord = co]ord.xy;")) else {
        panic!("accepted");
    };
    assert_eq!(
        message,
        "\nunexpected \"]\"\nexpecting letter or digit, \"_\", \"/\", space, \"[\", \".\", \
         \"++\", \"--\", operator, \"?\", \"=\" or \";\""
    );
}
