-- Pure let binding read once: UsedOnce candidate.
f x = let dead = 5 in dead + x
