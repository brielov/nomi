//! A shader's inputs (`Parse/Shader.hs`'s `extractInputs`): each
//! top-level declaration of one attribute, uniform or varying of a type
//! WebGL passes, by name. Upstream reads them from language-glsl's tree;
//! the shader parsed already (the syntax crate checks it as elm does), so
//! a scan of its top-level declarations finds the same ones.
//!
//! Upstream takes a declaration whose only qualifier is the storage one,
//! with an optional precision, one of seven types, and one name, maybe
//! sized or initialized; anything else declares nothing it passes.

use crate::canonical::{ShaderType, ShaderTypes};

/// The inputs of the shader `source`, the text between `[glsl|` and `|]`.
pub fn types(source: &str) -> ShaderTypes {
    let mut types = ShaderTypes::default();
    for declaration in top_level(&tokens(source)) {
        let [qualifier, rest @ ..] = &declaration[..] else {
            continue;
        };
        let rest = match rest {
            [precision, rest @ ..] if matches!(*precision, "lowp" | "mediump" | "highp") => rest,
            rest => rest,
        };
        let [ty, name, after @ ..] = rest else {
            continue;
        };
        // One declarator: sized or initialized, but no second name.
        if after.contains(&",") || !is_name(name) {
            continue;
        }
        let ty = match *ty {
            "vec2" => ShaderType::V2,
            "vec3" => ShaderType::V3,
            "vec4" => ShaderType::V4,
            "mat4" => ShaderType::M4,
            "int" => ShaderType::Int,
            "float" => ShaderType::Float,
            "sampler2D" => ShaderType::Texture,
            _ => continue,
        };
        let table = match *qualifier {
            "attribute" => &mut types.attribute,
            "uniform" => &mut types.uniform,
            "varying" => &mut types.varying,
            _ => continue,
        };
        table.insert((*name).to_owned(), ty);
    }
    types
}

fn is_name(token: &str) -> bool {
    token.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

/// The source's tokens: words and numbers whole, other characters one by
/// one, without comments or preprocessor lines.
fn tokens(source: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut line_start = true;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if (line_start && c == b'#') || source[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if source[i..].starts_with("/*") {
            i = source[i + 2..]
                .find("*/")
                .map_or(bytes.len(), |end| i + 2 + end + 2);
        } else if c.is_ascii_alphanumeric() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push(&source[start..i]);
            line_start = false;
        } else {
            let width = source[i..].chars().next().map_or(1, char::len_utf8);
            tokens.push(&source[i..i + width]);
            i += width;
            line_start = false;
        }
    }
    tokens
}

/// The declarations outside every brace: the tokens before each `;`.
fn top_level<'s>(tokens: &[&'s str]) -> Vec<Vec<&'s str>> {
    let mut declarations = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0usize;
    let mut parens = 0usize;
    for &token in tokens {
        match token {
            "{" => {
                depth += 1;
                current.clear();
            }
            "}" => {
                depth = depth.saturating_sub(1);
                current.clear();
            }
            "(" if depth == 0 => {
                parens += 1;
                current.push(token);
            }
            ")" if depth == 0 => {
                parens = parens.saturating_sub(1);
                current.push(token);
            }
            ";" if depth == 0 && parens == 0 => declarations.push(std::mem::take(&mut current)),
            _ if depth == 0 => current.push(token),
            _ => {}
        }
    }
    declarations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_are_the_single_storage_declarations_of_passable_types() {
        let source = "
precision mediump float;
#define X 1
attribute vec3 position; // a comment
uniform highp mat4 perspective;
uniform sampler2D texture;
varying vec4 vcolor;
uniform vec3 lights[4];
uniform float a, b;
const float c = 1.0;
uniform bool flag;
/* uniform int hidden; */
void main () { gl_Position = perspective * vec4(position, 1.0); }
";
        let types = types(source);
        let names = |m: &std::collections::BTreeMap<String, ShaderType>| -> Vec<String> {
            m.keys().cloned().collect()
        };
        assert_eq!(names(&types.attribute), ["position"]);
        assert_eq!(names(&types.uniform), ["lights", "perspective", "texture"]);
        assert_eq!(names(&types.varying), ["vcolor"]);
        assert_eq!(types.uniform["texture"], ShaderType::Texture);
    }
}
