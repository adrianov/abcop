-- Case, let, do-bind, conditional and a call: one dense unit.
branchy flag opt = do
  let total = case opt of
        Just n -> n + 1
        Nothing -> 0
  v <- pure total
  if flag == True then print v else pure ()
