module Main exposing (..)

import Json.Decode exposing (Decoder, int)


sumDecoder : Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (Json.Decode.field "x" int)
        (Json.Decode.field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 , int
-- INSERT , field
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (Decoder, field, int)
--
--
-- sumDecoder : Decoder Int
-- sumDecoder =
--     Json.Decode.map2 (+)
--         (field "x" int)
--         (field "y" int)
