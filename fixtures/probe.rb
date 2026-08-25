require 'json'

class Greeter
  attr_accessor :name
  def initialize(name)
    @name = name
  end
  def greet(greeting = "Hello", shout: false)
    msg = "#{greeting}, #{@name}!"
    shout ? msg.upcase : msg
  end
end

def compute(items, factor)
  total = 0
  items.each_with_index do |item, i|
    next if item.nil?
    v = item * factor
    total += v unless v < 10
  end
  total / factor
end

x = compute([1, nil, 20], 2)
y = x&.to_s&.length
h = { a: 1 }
a, *rest = [1, 2, 3]
m = h.map(&:to_s).map { |kv| kv }
n = h.any? { |v| v > 0 } && y || nil
case m
when Array then p m.length
else p :other
end
case x
in Integer then p :int
end
begin
  risky = JSON.parse("")
rescue => e
  warn e.message
ensure
  p :done
end
while x > 0 do x -= 1 end
until y.nil?
  y = nil
end
for i in 0..3
  p i
end
->(q) { q.to_i }
square = ->(w) { w * w }
def outer(a)
  def inner(b) = b + 1
  inner(a) rescue 0
  [1].map { |z| z + a }
end
obj&.configure do |cfg|
  cfg.mode = :fast
end
p defined?(x)
