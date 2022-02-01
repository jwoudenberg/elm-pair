module Main exposing (Boardgame, mkGame)


type Fun
    = Hiking
    | Reading
    | Game


type alias Boardgame =
    { name : String
    , maxPlayers : Int
    }


mkGame : String -> Int -> Boardgame
mkGame =
    Boardgame



-- START SIMULATION
-- MOVE CURSOR TO LINE 18 Boardgame
-- DELETE Boardgame
-- INSERT Game
-- END SIMULATION
-- === expected output below ===
-- No refactor for this change.
