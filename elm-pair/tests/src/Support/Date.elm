module Support.Date exposing (Clock(..), Date, now)


type Clock
    = Watch
    | Alarm
    | Grandfather


type Date
    = Yesterday
    | Today
    | Tomorrow


now : Date
now =
    Today
