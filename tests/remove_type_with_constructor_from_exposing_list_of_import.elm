module Main exposing (..)

import Url exposing (Protocol(..))


toggleSecure : Protocol -> Protocol
toggleSecure protocol =
    case protocol of
        Http ->
            Https

        Https ->
            Http



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 Protocol
-- DELETE Protocol(..)
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Url
--
--
-- toggleSecure : Url.Protocol -> Url.Protocol
-- toggleSecure protocol =
--     case protocol of
--         Url.Http ->
--             Url.Https
--         Url.Https ->
--             Url.Http
