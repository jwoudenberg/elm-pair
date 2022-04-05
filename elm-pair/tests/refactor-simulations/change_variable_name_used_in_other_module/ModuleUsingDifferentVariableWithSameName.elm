module ModuleUsingDifferentVariableWithSameName exposing (..)

import ModuleExposingVariable


greeting : String
greeting =
    "Gruezi"



-- === expected output below ===
-- No refactor for this change.
