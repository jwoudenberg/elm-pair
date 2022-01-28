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
-- MOVE CURSOR TO LINE 1 Boardgame
-- DELETE Boardgame
-- INSERT Game
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (Game, mkGame)
--
--
-- type Fun
--     = Hiking
--     | Reading
--     | Game2
--
--
-- type alias Game =
--     { name : String
--     , maxPlayers : Int
--     }
--
--
-- mkGame : String -> Int -> Game
-- mkGame =
--     Game
