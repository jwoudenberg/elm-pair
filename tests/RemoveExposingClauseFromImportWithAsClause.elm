module Math exposing (..)

import Json.Decode as Dec exposing (Decoder, field, int)


sumDecoder : Decoder Int
sumDecoder =
    Dec.map2 (+)
        (field "x" int)
        (field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 exposing
-- DELETE exposing (Decoder, field, int)
-- END SIMULATION
-- === expected output below ===
-- module Math exposing (..)
-- 
-- import Json.Decode as Dec 
-- 
-- 
-- sumDecoder : Dec.Decoder Int
-- sumDecoder =
--     Dec.map2 (+)
--         (Dec.field "x" Dec.int)
--         (Dec.field "y" Dec.int)
-- 
-- 
