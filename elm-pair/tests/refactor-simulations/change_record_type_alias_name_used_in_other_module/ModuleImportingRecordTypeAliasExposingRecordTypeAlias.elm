module ModuleImportingRecordTypeAliasExposingRecordTypeAlias exposing (..)

import ModuleExposingRecordTypeAlias exposing (Person)


jane : Person
jane =
    Person "Jane" 33



-- === expected output below ===
-- module ModuleImportingRecordTypeAliasExposingRecordTypeAlias exposing (..)
--
-- import ModuleExposingRecordTypeAlias exposing (Friend)
--
--
-- jane : Friend
-- jane =
--     Friend "Jane" 33
