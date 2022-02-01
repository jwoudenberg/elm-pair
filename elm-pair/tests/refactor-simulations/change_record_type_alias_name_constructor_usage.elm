module Main exposing (Boardgame, mkGame)


type alias Boardgame =
    { name : String
    , maxPlayers : Int
    }


mkGame : String -> Int -> Boardgame
mkGame =
    Boardgame



-- START SIMULATION
-- MOVE CURSOR TO LINE 12 Boardgame
-- DELETE Boardgame
-- INSERT Game
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (Game, mkGame)
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
