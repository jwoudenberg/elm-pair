module ModuleExposingRecordTypeAlias exposing (Person)


type alias Person =
    { name : String
    , age : Int
    }



-- === expected output below ===
-- module ModuleExposingRecordTypeAlias exposing (Friend)
--
--
-- type alias Friend =
--     { name : String
--     , age : Int
--     }
