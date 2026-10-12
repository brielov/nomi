
module Main exposing (..)

import Json.Encode


type alias Scope =
  { user : User
  }


type alias User =
  { name : String
  }


encodeScope : Scope -> Json.Encode.Value
encodeScope scope =
  Json.Encode.object
  [ ( "tag", Json.Encode.string "Scope" )
  , ( "user", encodeUser scope.user )
  ]


  