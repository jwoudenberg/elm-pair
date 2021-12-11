module Math exposing (..)

import Json.Decode as Dec exposing (Decoder, field, int)


sumDecoder : Decoder Int
sumDecoder =
    Dec.map2 (+)
        (field "x" int)
        (field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 , field
-- DELETE , field
-- END SIMULATION
-- === expected output below ===
-- module Math exposing (..)
-- 
-- import Json.Decode as Dec exposing (Decoder, int)
-- 
-- 
-- sumDecoder : Decoder Int
-- sumDecoder =
--     Dec.map2 (+)
--         (Dec.field "x" int)
--         (Dec.field "y" int)
-- 
-- 
