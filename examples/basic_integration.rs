use ruwren::{VM, Class, PrintlnPrinter, NullLoader, ModuleLibrary, create_module, get_slot_checked, send_foreign, Executor};

struct Vector {
    x: f64,
    y: f64
}

impl Class for Vector {
    fn initialize(_: &VM) -> Self {
        panic!("Cannot initialize from Wren code");
    }
}

impl Vector {
    fn x(&self, vm: &VM) {
        vm.ensure_slots(1);
        vm.set_slot_double(0, self.x);
    }

    fn y(&self, vm: &VM) {
        vm.ensure_slots(1);
        vm.set_slot_double(0, self.y);
    }

    fn set_x(&mut self, vm: &VM) {
        vm.ensure_slots(2);
        self.x = get_slot_checked!(vm => num 1);
    }

    fn set_y(&mut self, vm: &VM) {
        vm.ensure_slots(2);
        self.y = get_slot_checked!(vm => num 1);
    }
}

struct Math;

impl Class for Math {
    fn initialize(_: &VM) -> Self {
        panic!("Math is a purely static class");
    }
}

impl Math {
    fn new_vector(vm: &VM) {
        vm.ensure_slots(3);
        let x = get_slot_checked!(vm => num 1);
        let y = get_slot_checked!(vm => num 2);
        send_foreign!(vm, "maths", "Vector", Vector { x, y } => 0);
    }
}

create_module!(
    class("Vector") crate::Vector => vector {
        instance("x()") x,
        instance("y()") y,
        instance("set_x(_)") set_x,
        instance("set_y(_)") set_y
    }

    class("Math") crate::Math => math {
        static("new_vector(_,_)") new_vector
    }

    module => maths
);

static MATHS_MODULE_SRC: &'static str = "
foreign class Vector {
    construct invalid() {}
    foreign x()
    foreign y()
    foreign set_x(x)
    foreign set_y(y)
}

class Math {
    foreign static new_vector(x, y)
}
";

fn main() {
    let mut lib = ModuleLibrary::new();
    maths::publish_module(&mut lib);
    let vm = VM::new(PrintlnPrinter, NullLoader, Some(&lib));
    vm.execute(|vm| {
        vm.interpret("maths", MATHS_MODULE_SRC).unwrap(); // Should succeed
    });

    vm.execute(|vm| {
        let res = vm.interpret("main", "
        import \"maths\" for Math, Vector
        var poke_vector = Fiber.new {
            Vector.invalid()
        }
        System.print(\"Hello World\")
        System.print(poke_vector.try())
        var vector = Math.new_vector(3, 4)
        System.print(vector)
        System.print(vector.x())
        System.print(vector.y())
        vector.set_x(10.2)
        vector.set_y(vector.x() * 2)
        System.print(vector.x())
        System.print(vector.y())
        ");

        if let Err(err) = res {
            eprintln!("{}", err);
        }
    })
}