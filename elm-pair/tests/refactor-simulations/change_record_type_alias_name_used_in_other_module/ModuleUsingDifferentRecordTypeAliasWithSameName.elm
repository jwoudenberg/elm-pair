module ModuleUsingDifferentRecordTypeAliasWithSameName exposing (..)

import ModuleExposingRecordTypeAlias


type alias Person =
    { id : Int
    , name : String
    }



-- === expected output below ===
-- No refactor for this change.
