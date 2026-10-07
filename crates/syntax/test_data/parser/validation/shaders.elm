module M exposing (..)


fine =
    [glsl|
precision mediump float;
void main () { gl_FragColor = vec4(1.0); }
|]


shifted =
    [glsl| void main () { x <<= 2; } |]


octal =
    [glsl| float f = 08.5; |]
