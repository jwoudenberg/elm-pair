module Main exposing (..)

import Json.Decode exposing (field, int)


sumDecoder : Json.Decode.Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (field "x" int)
        (field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 6 Json.
-- DELETE Json.Decode.
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
