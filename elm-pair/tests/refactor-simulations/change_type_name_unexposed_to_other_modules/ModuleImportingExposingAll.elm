module ModuleImportingExposingAll exposing (..)

import ModuleNotExposingType exposing (..)


type Snore
    = Snore


snore : Snore
snore =
    Snore



-- === expected output below ===
-- No refactor for this change.
