-- Where-bound function is its own AbcSize unit.
outer x = helper x
  where
    helper y = y + 1
