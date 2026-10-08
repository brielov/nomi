-- What the solver oracle needs that 0.19.2's sources spell differently
-- from 0.19.3's. The oracles for 0.19.1 and 0.19.2 build on these.
module Compat (Cache, getPackageCache, readOutline) where

import qualified Elm.Outline as Outline
import qualified Stuff


type Cache = Stuff.PackageCache


getPackageCache :: IO Cache
getPackageCache =
  Stuff.getPackageCache


-- The project in the current directory.
readOutline :: IO (Maybe Outline.Outline)
readOutline =
  either (const Nothing) Just <$> Outline.read "."
