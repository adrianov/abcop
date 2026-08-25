def probe(p, h, cfg)
  q = p[:id]
  s = settings
  r = h['k']
  t = cfg.fetch(:x, {})
  u = Foo::BAR
  super
  super()
  v = not q
  w = y = 1
  n = h.each_pair { |(kk, vv)| kk + vv }
  m = [1].each { |(a2, b2)| a2 }
  o = q ? 1 : 2
  h2 = { a: q }
  str = "v#{q} #{s}"
  z = p[0] && h[1]
  q&.foo
  defined?(q)
  p.instance_variable_get(:@z)
  [q, s].include?(t)
end
