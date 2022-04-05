module ModuleNotExposingRecordTypeAlias exposing (create)


type alias Person =
    { name : String
    , age : Int
    }


create : String -> Int -> Person
create =
    Person



-- === expected output below ===
-- module ModuleNotExposingRecordTypeAlias exposing (create)
--
--
-- type alias Friend =
--     { name : String
--     , age : Int
--     }
--
--
-- create : String -> Int -> Friend
-- create =
--     Friend
