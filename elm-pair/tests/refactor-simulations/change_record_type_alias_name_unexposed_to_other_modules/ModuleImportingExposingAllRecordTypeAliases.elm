module ModuleImportingExposingAllRecordTypeAliases exposing (..)

import ModuleNotExposingRecordTypeAlias


type alias Person =
    { id : Int
    , name : String
    }



-- === expected output below ===
-- No refactor for this change.
