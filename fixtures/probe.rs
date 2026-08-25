struct P {
    a: u32,
}

fn compute(items: &[Option<u32>], factor: u32) -> u32 {
    let mut total = 0u32;
    for item in items.iter() {
        if item.is_none() {
            continue;
        }
        total += item.unwrap() * factor;
    }
    total / factor
}

impl P {
    fn get(&self) -> u32 {
        self.a
    }
    fn set(&mut self, v: u32) {
        self.a = v;
    }
}

fn cond(x: i32) -> i32 {
    if x == 1 && x < 5 {
        1
    } else if x > 10 {
        2
    } else {
        3
    }
}

fn mat(c: u8) -> &'static str {
    match c {
        0 => "zero",
        1..=3 => "small",
        n if n > 10 => "big",
        _ => "other",
    }
}

fn closures(v: Vec<u32>) -> u32 {
    let doubled: Vec<u32> = v.iter().map(|x| x * 2).collect();
    let add = |a: u32| a + doubled.len() as u32;
    let half = &doubled;
    add(half.len() as u32)
}

fn macros() {
    println!("{} {}", 1, 2);
    let v = vec![1, 2, 3];
    assert_eq!(v.len(), 3);
}

fn try_op(x: Result<u32, ()>) -> Result<u32, ()> {
    let y = x?;
    Ok(y + 1)
}

fn shadow_and_mut() {
    let mut n = 1;
    n += 1;
    let n = n * 2;
    println!("{}", n);
}
