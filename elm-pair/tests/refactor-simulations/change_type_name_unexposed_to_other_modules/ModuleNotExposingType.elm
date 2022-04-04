module ModuleNotExposingType exposing (sleep)


type Snore
    = Zzz


sleep : Snore
sleep =
    Zzz



-- === expected output below ===
-- module ModuleNotExposingType exposing (sleep)
--
--
-- type SleepySounds
--     = Zzz
--
--
-- sleep : SleepySounds
-- sleep =
--     Zzz
