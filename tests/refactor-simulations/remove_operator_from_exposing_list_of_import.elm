module Main exposing (..)

import Parser exposing ((|=), Parser, int)


add : Parser Int
add =
    Parser.succeed (+)
        |= int
        |= int



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 (|=)
-- DELETE (|=), Parser,
-- END SIMULATION
-- === expected output below ===
-- No refactor for this change.
