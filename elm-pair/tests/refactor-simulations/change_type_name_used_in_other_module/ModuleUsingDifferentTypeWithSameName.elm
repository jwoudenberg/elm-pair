module ModuleUsingDifferentTypeWithSameName exposing (..)

import ModuleExposingType


type Snore
    = Snore



-- === expected output below ===
-- No refactor for this change.
