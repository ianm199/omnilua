use std::mem::{align_of, size_of};

use lua_types::{
    CallInfoIdx, GcRef, LuaClosure, LuaLClosure, LuaProto, LuaString, LuaTable, LuaUserData,
    LuaValue, StackIdx, UpVal,
};
use lua_vm::state::{CallInfo, CallInfoExtra, CallInfoFrame, LuaState, StackValue};

fn row(name: &str, size: usize, align: usize) {
    println!("rust\t{name}\t{size}\t{align}");
}

fn main() {
    println!("impl\ttype\tsize_bytes\talign_bytes");
    row("LuaValue", size_of::<LuaValue>(), align_of::<LuaValue>());
    row(
        "StackValue",
        size_of::<StackValue>(),
        align_of::<StackValue>(),
    );
    row("CallInfo", size_of::<CallInfo>(), align_of::<CallInfo>());
    row(
        "CallInfoFrame",
        size_of::<CallInfoFrame>(),
        align_of::<CallInfoFrame>(),
    );
    row(
        "CallInfoExtra",
        size_of::<CallInfoExtra>(),
        align_of::<CallInfoExtra>(),
    );
    row("LuaState", size_of::<LuaState>(), align_of::<LuaState>());
    row("StackIdx", size_of::<StackIdx>(), align_of::<StackIdx>());
    row(
        "CallInfoIdx",
        size_of::<CallInfoIdx>(),
        align_of::<CallInfoIdx>(),
    );
    row(
        "GcRef<LuaString>",
        size_of::<GcRef<LuaString>>(),
        align_of::<GcRef<LuaString>>(),
    );
    row(
        "GcRef<LuaTable>",
        size_of::<GcRef<LuaTable>>(),
        align_of::<GcRef<LuaTable>>(),
    );
    row("LuaString", size_of::<LuaString>(), align_of::<LuaString>());
    row("LuaTable", size_of::<LuaTable>(), align_of::<LuaTable>());
    row(
        "LuaClosure",
        size_of::<LuaClosure>(),
        align_of::<LuaClosure>(),
    );
    row(
        "LuaLClosure",
        size_of::<LuaLClosure>(),
        align_of::<LuaLClosure>(),
    );
    row("LuaProto", size_of::<LuaProto>(), align_of::<LuaProto>());
    row(
        "LuaUserData",
        size_of::<LuaUserData>(),
        align_of::<LuaUserData>(),
    );
    row("UpVal", size_of::<UpVal>(), align_of::<UpVal>());
}
