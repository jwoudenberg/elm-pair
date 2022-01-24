module Main exposing (..)

import Json.Decode exposing (..)


decodeSum : Decoder Int
decodeSum =
    let
        x =
            "x"

        y =
            "y"
    in
    map2 (+) (field x int) (field y int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 9 x
-- DELETE x
-- INSERT field
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (Decoder, int, map2)
--
--
-- decodeSum : Decoder Int
-- decodeSum =
--     let
--         field =
--             "x"
--
--         y =
--             "y"
--     in
--     map2 (+) (Json.Decode.field field int) (Json.Decode.field y int)
