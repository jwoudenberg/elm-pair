module Main exposing (..)

import Json.Decode exposing (int)


type alias Decoder =
    Json.Decode.Decoder Int


sumDecoder : Decoder
sumDecoder =
    Json.Decode.map2 (+)
        (Json.Decode.field "x" int)
        (Json.Decode.field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 7 Json.Decode.Decoder
-- DELETE Json.Decode.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (Decoder, int)
--
--
-- type alias Decoder2 =
--     Decoder Int
--
--
-- sumDecoder : Decoder2
-- sumDecoder =
--     Json.Decode.map2 (+)
--         (Json.Decode.field "x" int)
--         (Json.Decode.field "y" int)
