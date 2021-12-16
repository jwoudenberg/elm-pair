module Main exposing (..)

import Json.Decode exposing (..)


sumDecoder : Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (field "x" int)
        (field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 9 field
-- INSERT Json.Decode.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (..)
--
--
-- sumDecoder : Decoder Int
-- sumDecoder =
--     Json.Decode.map2 (+)
--         (Json.Decode.field "x" int)
--         (Json.Decode.field "y" int)
