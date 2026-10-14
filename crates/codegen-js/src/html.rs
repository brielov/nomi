//! `Generate/Html.hs`: a program's JavaScript in a page that starts its
//! one `main`, and shows what goes wrong starting it. All three releases
//! write the page alike.

/// `sandwich`: the page of `module`'s program, whose code is `javascript`.
pub fn sandwich(module: &str, javascript: &[u8]) -> Vec<u8> {
    [
        b"<!DOCTYPE HTML>\n<html>\n<head>\n  <meta charset=\"UTF-8\">\n  <title>".as_slice(),
        module.as_bytes(),
        b"</title>\n  <style>body { padding: 0; margin: 0; }</style>\n</head>\n\n<body>\n\n\
          <pre id=\"elm\"></pre>\n\n<script>\ntry {\n",
        javascript,
        b"\n\n  var app = Elm.",
        module.as_bytes(),
        b".init({ node: document.getElementById(\"elm\") });\n}\n\
          catch (e)\n{\n  // display initialization errors (e.g. bad flags, infinite recursion)\n  \
          var header = document.createElement(\"h1\");\n  header.style.fontFamily = \"monospace\";\n  \
          header.innerText = \"Initialization Error\";\n  var pre = document.getElementById(\"elm\");\n  \
          document.body.insertBefore(header, pre);\n  pre.innerText = e;\n  throw e;\n}\n</script>\n\n\
          </body>\n</html>",
    ]
    .concat()
}
