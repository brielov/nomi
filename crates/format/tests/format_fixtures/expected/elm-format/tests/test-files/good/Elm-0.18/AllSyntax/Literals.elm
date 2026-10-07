module AllSyntax.Literals exposing (boolean, char, escapedControlChars, escapedNonPrintChars, escapedSpecialChars, multilineString, multilineStringQuoteHandling, string, stringWithEscapedControlCharacters, stringWithEscapedNonPrintCharacters, stringWithSpecialEscapedCharacters, stringWithUnicodeCharacters, stringWithWhitespace, stringWithWindowsWorkaroundCharacters, unicodeChars, unit, whitespaceChars)


unit =
    ()


boolean =
    [ True
    , False
    ]


integer =
    [ 1
    , 0x0F1A
    , 0x7FFFFFFFFFFFFFFF
    , 9223372036854775807
    , -9223372036854775808
    ]


float =
    [ 2.0
    ]


string =
    "string"


stringWithUnicodeCharacters =
    "a b'`<>{}/ڥ😀ぁ⾃𝟟"


stringWithEscapedCharacterFollowedByHexDigit =
    "😀0😀9😀a😀F"


stringWithSpecialEscapedCharacters =
    "\t\n\\\""


stringWithEscapedControlCharacters =
    [ "\u{0000}\u{0001}\u{0002}\u{0003}\u{0004}\u{0005}\u{0006}\u{0007}\u{0008}\u{000B}\u{000C}\u{000D}\u{000E}\u{000F}"
    , "\u{0010}\u{0011}\u{0012}\u{0013}\u{0014}\u{0015}\u{0016}\u{0017}\u{0018}\u{0019}\u{001A}\u{001B}\u{001C}\u{001D}\u{001E}\u{001F}"
    , "\u{007F}"
    ]


stringWithWindowsWorkaroundCharacters =
    -- See https://github.com/avh4/elm-format/issues/134
    "\u{00A0}"


stringWithEscapedNonPrintCharacters =
    "\u{06DD}\u{FFFB}\u{110BD}\u{E007F}"


stringWithWhitespace =
    "\u{00A0}\u{2000}\u{205F}"


char =
    'c'


unicodeChars =
    [ 'a', ' ', '/', '"', 'ڥ', '😀', 'ぁ', '⾃', '𝟟' ]


escapedSpecialChars =
    [ '\t', '\n', '\\', '\'' ]


escapedControlChars =
    [ '\u{0000}', '\u{001F}', '\u{007F}' ]


escapedNonPrintChars =
    [ '\u{06DD}', '\u{FFFB}', '\u{110BD}', '\u{E007F}' ]


whitespaceChars =
    [ '\u{00A0}', '\u{2000}', '\u{205F}' ]


multilineString =
    """normals = "a b'`<>{}/ڥ😀ぁ⾃𝟟"
"\t\\
""
\"\"\"
\"\"\"\"
\u{0000}\u{0001}\u{0002}\u{0003}\u{0004}\u{0005}\u{0006}\u{0007}\u{0008}\u{000B}\u{000C}\u{000D}\u{000E}\u{000F}
\u{0010}\u{0011}\u{0012}\u{0013}\u{0014}\u{0015}\u{0016}\u{0017}\u{0018}\u{0019}\u{001A}\u{001B}\u{001C}\u{001D}\u{001E}\u{001F}
\u{007F}
\u{06DD}\u{FFFB}\u{110BD}\u{E007F}
\u{00A0}\u{2000}\u{205F}
"""


multilineStringQuoteHandling =
    [ """trailing double quote\""""
    , """trailing double quote 2\"\""""
    , """trailing double quote 3\"\"\""""
    , """trailing double quote 4\"\"\"\""""
    , """embedded triple double quotes (\"\"\")"""
    , """embedded triple double quotes (\"\"\") (\"\"\"\")"""
    , """""leading double quote 2"""
    , """\"\"\" leading double quote 3"""
    , """\"\"\"\" leading double quote 4"""
    ]
