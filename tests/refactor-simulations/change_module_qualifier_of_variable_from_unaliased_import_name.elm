module Main exposing (..)

import Json.Decode exposing (int)


sumDecoder : Json.Decode.Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (Json.Decode.field "x" int)
        (Json.Decode.field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 8 Json.Decode
-- DELETE Json.Decode
-- INSERT Dec
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode as Dec exposing (int)
--
--
-- sumDecoder : Dec.Decoder Int
-- sumDecoder =
--     Dec.map2 (+)
--         (Dec.field "x" int)
--         (Dec.field "y" int)
