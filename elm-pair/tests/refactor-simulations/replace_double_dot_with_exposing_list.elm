module Main exposing (..)

import Json.Decode exposing (..)


sumDecoder : Decoder Int
sumDecoder =
    map2 (+)
        (field "x" int)
        (field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 ..
-- DELETE ..
-- INSERT Decoder, map2
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (Decoder, map2)
--
--
-- sumDecoder : Decoder Int
-- sumDecoder =
--     map2 (+)
--         (Json.Decode.field "x" Json.Decode.int)
--         (Json.Decode.field "y" Json.Decode.int)
