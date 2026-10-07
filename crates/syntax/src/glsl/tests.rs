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
    assert_eq!(check(""), Err(Rejection::At(0)));
    assert_eq!(check("  // nothing"), Err(Rejection::At(12)));
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
    assert_eq!(check("void main () { x = ; }"), Err(Rejection::At(19)));
    assert_eq!(check("void main () { /* open }"), Err(Rejection::At(24)));
    assert_eq!(check("int x = 1;\nint __y;"), Err(Rejection::At(18)));
}

#[test]
fn deep_nesting_is_rejected() {
    let deep = format!("int x = {}1{};", "(".repeat(100_000), ")".repeat(100_000));
    assert!(matches!(check(&deep), Err(Rejection::TooDeep(_))));
}
