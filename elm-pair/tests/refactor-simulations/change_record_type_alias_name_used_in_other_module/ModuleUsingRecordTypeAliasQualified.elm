module ModuleUsingRecordTypeAliasQualified exposing (..)

import ModuleExposingRecordTypeAlias


jane : ModuleExposingRecordTypeAlias.Person
jane =
    ModuleExposingRecordTypeAlias.Person "Jane" 33



-- === expected output below ===
-- module ModuleUsingRecordTypeAliasQualified exposing (..)
--
-- import ModuleExposingRecordTypeAlias
--
--
-- jane : ModuleExposingRecordTypeAlias.Friend
-- jane =
--     ModuleExposingRecordTypeAlias.Friend "Jane" 33
