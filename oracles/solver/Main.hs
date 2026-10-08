{-# LANGUAGE OverloadedStrings #-}
-- The solver oracle: upstream's own `Deps/Solver.hs`, driven as `elm make`
-- and `elm install` drive it, over a fixture registry and package cache.
--
--   ELM_HOME=<home> solver-oracle <registry> [<package>]
--
-- runs in a project directory. `<registry>` lists one package per line,
-- `author/project v1 v2 ...`, newest first as `KnownVersions` keeps them.
-- The cache is `$ELM_HOME/<compiler>/packages`, where upstream keeps
-- release metadata. The solver runs offline, so it reads nothing else.
--
-- Without a package, the oracle verifies the project's dependencies as
-- `Elm/Details.hs` does (`verifyPkg`, `verifyApp`). With one, it plans its
-- installation as `Install.hs` does (`makeAppPlan`, `makePkgPlan`). It
-- prints `ok` and the solution, or what went wrong.
module Main (main) where

import qualified Data.ByteString.UTF8 as BS_UTF8
import qualified Data.Map as Map
import System.Environment (getArgs)
import System.Exit (die)
import System.IO (hSetEncoding, stdout, utf8)

import qualified Compat
import qualified Deps.Registry as Registry
import qualified Deps.Solver as Solver
import qualified Elm.Constraint as C
import qualified Elm.Outline as Outline
import qualified Elm.Package as Pkg
import qualified Elm.Version as V
import qualified Parse.Primitives as P
import qualified Reporting.Annotation as A



-- MAIN


main :: IO ()
main =
  do  hSetEncoding stdout utf8
      args <- getArgs
      (registryPath, add) <-
        case args of
          [path]      -> return (path, Nothing)
          [path, pkg] -> (,) path . Just <$> parseName pkg
          _           -> die "usage: solver-oracle <registry> [<package>]"
      registry <- readRegistry registryPath
      cache <- Compat.getPackageCache
      maybeOutline <- Compat.readOutline
      lines_ <-
        case maybeOutline of
          Nothing ->
            return ["bad-outline"]

          Just outline ->
            case add of
              Nothing  -> verify cache registry outline
              Just pkg -> install cache registry pkg outline
      putStr (unlines lines_)



-- VERIFY, as Elm.Details does


verify :: Compat.Cache -> Registry.Registry -> Outline.Outline -> IO [String]
verify cache registry outline =
  case outline of
    Outline.Pkg (Outline.PkgOutline _ _ _ _ _ direct testDirect elm) ->
      if not (C.goodElm elm) then
        return ["bad-elm"]
      else
        case unionNoDups direct testDirect of
          Nothing ->
            return ["hand-edited"]

          Just constraints ->
            do  result <- Solver.verify cache Solver.Offline registry constraints
                return $ withResult result $ \solution ->
                  "ok" : map showRelease (Map.toList (Map.map detailsVersion solution))

    Outline.App (Outline.AppOutline elm _ direct indirect testDirect testIndirect) ->
      if elm /= V.compiler then
        return ["bad-elm"]
      else
        -- checkAppDeps
        case unionWith (==) indirect testDirect of
          Nothing ->
            return ["hand-edited"]

          Just x ->
            case unionNoDups direct testIndirect >>= unionNoDups x of
              Nothing ->
                return ["hand-edited"]

              Just stated ->
                do  result <- Solver.verify cache Solver.Offline registry (Map.map C.exactly stated)
                    return $ withResult result $ \solution ->
                      if Map.size stated == Map.size solution
                      then "ok" : map showRelease (Map.toList (Map.map detailsVersion solution))
                      else ["hand-edited"]


detailsVersion :: Solver.Details -> V.Version
detailsVersion (Solver.Details vsn _) =
  vsn


unionNoDups :: (Ord k) => Map.Map k v -> Map.Map k v -> Maybe (Map.Map k v)
unionNoDups =
  unionWith (\_ _ -> False)


unionWith :: (Ord k) => (v -> v -> Bool) -> Map.Map k v -> Map.Map k v -> Maybe (Map.Map k v)
unionWith allowed a b =
  if and (Map.elems (Map.intersectionWith allowed a b))
  then Just (Map.union a b)
  else Nothing



-- INSTALL, as Install does


install :: Compat.Cache -> Registry.Registry -> Pkg.Name -> Outline.Outline -> IO [String]
install cache registry pkg outline =
  case outline of
    Outline.App app@(Outline.AppOutline _ _ direct indirect testDirect testIndirect)
      | Map.member pkg direct -> return ["already-installed"]
      | Map.member pkg indirect -> return ["promote-indirect"]
      | Map.member pkg testDirect -> return ["promote-test"]
      | Map.member pkg testIndirect -> return ["promote-test"]
      | not (known pkg registry) -> return ["unknown-package"]
      | otherwise ->
          do  result <- Solver.addToApp cache Solver.Offline registry pkg app
              return $ withResult result $ \(Solver.AppSolution _ _ new) ->
                case new of
                  Outline.AppOutline _ _ d i td ti ->
                    "ok"
                    : section "direct" d
                    ++ section "indirect" i
                    ++ section "test-direct" td
                    ++ section "test-indirect" ti

    Outline.Pkg (Outline.PkgOutline _ _ _ _ _ deps test _)
      | Map.member pkg deps -> return ["already-installed"]
      | Map.member pkg test -> return ["promote-test"]
      | not (known pkg registry) -> return ["unknown-package"]
      | otherwise ->
          do  let constraints = Map.insert pkg C.anything (Map.union deps test)
              result <- Solver.verify cache Solver.Offline registry constraints
              return $ withResult result $ \solution ->
                "ok" : map showRelease (Map.toList (Map.map detailsVersion solution))


known :: Pkg.Name -> Registry.Registry -> Bool
known pkg (Registry.Registry _ versions) =
  Map.member pkg versions


section :: String -> Map.Map Pkg.Name V.Version -> [String]
section label deps =
  map (\release -> label ++ " " ++ showRelease release) (Map.toList deps)



-- OUTPUT


withResult :: Solver.Result a -> (a -> [String]) -> [String]
withResult result toLines =
  case result of
    Solver.Ok a              -> toLines a
    Solver.NoSolution        -> ["no-solution"]
    Solver.NoOfflineSolution -> ["no-offline-solution"]
    Solver.Err _             -> ["error"]


showRelease :: (Pkg.Name, V.Version) -> String
showRelease (name, vsn) =
  Pkg.toChars name ++ " " ++ V.toChars vsn



-- REGISTRY


readRegistry :: FilePath -> IO Registry.Registry
readRegistry path =
  do  text <- readFile path
      entries <- mapM parseEntry (filter (not . null) (lines text))
      let count = sum (map (\(_, Registry.KnownVersions _ vs) -> 1 + length vs) entries)
      return (Registry.Registry count (Map.fromList entries))


parseEntry :: String -> IO (Pkg.Name, Registry.KnownVersions)
parseEntry line =
  case words line of
    name : newest : previous ->
      do  pkg <- parseName name
          vsn <- parseVersion newest
          vsns <- mapM parseVersion previous
          return (pkg, Registry.KnownVersions vsn vsns)

    _ ->
      die ("bad registry line: " ++ line)


parseName :: String -> IO Pkg.Name
parseName chars =
  do  result <- P.fromByteString Pkg.parser A.Position (BS_UTF8.fromString chars)
      either (\_ -> die ("bad package name: " ++ chars)) return result


parseVersion :: String -> IO V.Version
parseVersion chars =
  do  result <- P.fromByteString V.parser A.Position (BS_UTF8.fromString chars)
      either (\_ -> die ("bad version: " ++ chars)) return result
