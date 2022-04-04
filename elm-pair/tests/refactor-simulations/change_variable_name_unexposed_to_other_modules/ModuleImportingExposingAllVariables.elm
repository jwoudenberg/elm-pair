module ModuleImportingExposingAllVariables exposing (..)

import ModuleNotExposingVariable exposing (..)


greeting : String
greeting =
    "Hello"


greetWorld : String
greetWorld =
    greeting ++ ", World!"



-- === expected output below ===
-- No refactor for this change.
