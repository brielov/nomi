# Third-party notices

nomi ports behavior, algorithms and messages from the projects below. Their licenses apply to the ported parts and are reproduced verbatim. Packages and kernel code vendored later add their notices here.

## Elm compiler

Source: https://github.com/elm/compiler. Ported into the syntax, compiler, codegen-js, project, registry and cli crates, and the `elm` command-line behavior.

```text
Copyright 2012-present Evan Czaplicki

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## elm-format

Source: https://github.com/avh4/elm-format. Ported into the format crate and the `elm-format` command-line behavior. Five of its test files (`tests/test-files/good/Elm-0.19/`) seed the syntax mutation suite in `crates/syntax/tests/syntax_mutations/seeds/`. Its `tests/test-files/` and `parser/tests/test-files/` (without the CommonMark spec) are copied unchanged into `crates/format/tests/format_fixtures/inputs/elm-format/`, and elm-format's output on them is stored under `expected/`.

```text
Copyright (c) 2015-2023, Aaron VonderHaar
Copyright (c) 2012-2015, Evan Czaplicki (Elm parser forked from elm-lang/elm-compiler)
Copyright (c) 2015, Martin Janiczek
Copyright (c) 2015, Noah Hall
Copyright (c) 2016, Marica Odagaki
Copyright (c) 2016, Max Goldstein
Copyright (c) 2016, Daniel Bachler
Copyright (c) 2016, Fedor Nezhivoi
Copyright (c) 2013, John MacFarlane (Markdown parser forked from jgm/cheapskate)
Copyright (c) 2017, Michael Maloney
Copyright (c) 1999-2000, Daan Leijen. (part of Text.Parsec.Token)
Copyright (c) 2007, Paolo Martini. (part of Text.Parsec.Token)
Copyright (c) 2018, Andreas Scharf
Copyright (c) 2018, Matthias Devlamynck
Copyright (c) 2018, Juliano Bortolozzo Solanho
Copyright (c) 2020-2023, Flavio Corpa
Copyright (c) 2022, Simon Lydell


All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

    * Redistributions of source code must retain the above copyright
      notice, this list of conditions and the following disclaimer.

    * Redistributions in binary form must reproduce the above
      copyright notice, this list of conditions and the following
      disclaimer in the documentation and/or other materials provided
      with the distribution.

    * Neither the name of the copyright holder nor the names of other
      contributors may be used to endorse or promote products derived
      from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## language-glsl

Source: https://github.com/noteed/language-glsl. Its parser, which elm runs on shader blocks, ported into the syntax crate.

```text
Copyright: (c) Vo Minh Thu, 2010.

All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.
3. Neither the name of the author nor the names of his contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE CONTRIBUTORS ``AS IS'' AND ANY
EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHORS OR CONTRIBUTORS
BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR
BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE
OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN
IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Parsec

Source: https://github.com/haskell/parsec. Its error messages (`Text.Parsec.Error`, and the labels of the combinators language-glsl uses), which elm quotes for shader blocks, ported into the syntax crate.

```text
Copyright 1999-2000, Daan Leijen; 2007, Paolo Martini. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice,
  this list of conditions and the following disclaimer.
* Redistributions in binary form must reproduce the above copyright
  notice, this list of conditions and the following disclaimer in the
  documentation and/or other materials provided with the distribution.

This software is provided by the copyright holders "as is" and any express or
implied warranties, including, but not limited to, the implied warranties of
merchantability and fitness for a particular purpose are disclaimed. In no
event shall the copyright holders be liable for any direct, indirect,
incidental, special, exemplary, or consequential damages (including, but not
limited to, procurement of substitute goods or services; loss of use, data,
or profits; or business interruption) however caused and on any theory of
liability, whether in contract, strict liability, or tort (including
negligence or otherwise) arising in any way out of the use of this software,
even if advised of the possibility of such damage.
```

## Elm packages

`crates/compiler/tests/packages/` holds copies of these packages' sources, unmodified, for tests (plan 5.3). Each is under the BSD-3-Clause license, kept verbatim in the copy's `LICENSE` file:

- elm/browser 1.0.2: Copyright 2017-present Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/browser/1.0.2/LICENSE`.
- elm/bytes 1.0.8: Copyright (c) 2018-present, Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/bytes/1.0.8/LICENSE`.
- elm/core 1.0.5: Copyright 2014-present Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/core/1.0.5/LICENSE`.
- elm/file 1.0.5: Copyright (c) 2018-present, Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/file/1.0.5/LICENSE`.
- elm/html 1.0.1: Copyright (c) 2014-present, Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/html/1.0.1/LICENSE`.
- elm/http 2.0.0: Copyright (c) 2016-present, Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/http/2.0.0/LICENSE`.
- elm/json 1.1.4: Copyright 2014-present Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/json/1.1.4/LICENSE`.
- elm/time 1.0.0: Copyright (c) 2018-present, Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/time/1.0.0/LICENSE`.
- elm/url 1.0.0: Copyright (c) 2016, Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/url/1.0.0/LICENSE`.
- elm/virtual-dom 1.0.5: Copyright (c) 2016-present, Evan Czaplicki. Its license is `crates/compiler/tests/packages/elm/virtual-dom/1.0.5/LICENSE`.

## elm-language-server

Source: https://github.com/elm-tooling/elm-language-server. Ported into the ide and lsp crates. Its completion data, the package ranking and the snippets, is copied into `crates/ide/src/completion_data.rs`.

```text
MIT License

Copyright (c) 2019 Kolja Lampe

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
