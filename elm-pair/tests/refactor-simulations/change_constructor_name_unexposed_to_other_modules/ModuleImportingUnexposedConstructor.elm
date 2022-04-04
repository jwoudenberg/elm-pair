module ModuleImportingUnexposedConstructor exposing (..)

import ModuleNotExposingConstructor exposing (..)


type Zzz
    = Zzz


snore : Zzz
snore =
    Zzz



-- === expected output below ===
-- No refactor for this change.
