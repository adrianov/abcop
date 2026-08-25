def big(a)
  x1 = a.foo
  x2 = x1.bar
  x3 = x2.baz
  x4 = x3.qux
  x5 = x4.quux
  x6 = x5.corge
  [x1, x2, x3, x4, x5, x6].sum / 7
end

def inline_me
  tmp = 42
  p tmp
end
