-- Lambda rolls into the enclosing bind; applications count as B.
f xs = map (\x -> x + 1) xs
