module Main exposing (..)

import Json.Decode exposing (Decoder, int)


field : String
field =
    "x"


field2 : String
field2 =
    "y"


sumDecoder : Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (Json.Decode.field field int)
        (Json.Decode.field field2 int)



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
-- field3 : String
-- field3 =
--     "x"
--
--
-- field2 : String
-- field2 =
--     "y"
--
--
-- sumDecoder : Decoder Int
-- sumDecoder =
--     Json.Decode.map2 (+)
--         (field field3 int)
--         (field field2 int)
--
--
