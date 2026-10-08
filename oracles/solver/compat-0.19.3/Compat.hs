-- What the solver oracle needs that 0.19.3's sources spell differently
-- from 0.19.2's.
module Compat (Cache, getPackageCache, readOutline) where

import qualified Elm.Outline as Outline
import qualified Root as R


type Cache = R.PackageCache


getPackageCache :: IO Cache
getPackageCache =
  R.getPackageCache


-- The project in the current directory.
readOutline :: IO (Maybe Outline.Outline)
readOutline =
  do  root <- R.pwd
      either (const Nothing) Just <$> Outline.read root
