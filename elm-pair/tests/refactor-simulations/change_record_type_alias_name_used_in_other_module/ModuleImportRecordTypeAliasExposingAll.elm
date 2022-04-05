module ModuleImportingRecordTypeAliasExposingAll exposing (..)

import ModuleExposingRecordTypeAlias exposing (..)


jane : Person
jane =
    Person "Jane" 33



-- === expected output below ===
-- module ModuleImportingRecordTypeAliasExposingAll exposing (..)
--
-- import ModuleExposingRecordTypeAlias exposing (..)
--
--
-- jane : Friend
-- jane =
--     Friend "Jane" 33
