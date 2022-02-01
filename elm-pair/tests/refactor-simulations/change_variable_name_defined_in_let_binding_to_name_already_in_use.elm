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
-- MOVE CURSOR TO LINE 15 x
-- DELETE x
-- INSERT field
-- END SIMULATION
-- === expected output below ===
-- No refactor for this change.
