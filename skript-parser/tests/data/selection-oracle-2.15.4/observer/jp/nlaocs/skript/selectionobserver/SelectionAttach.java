package jp.nlaocs.skript.selectionobserver;

import com.sun.tools.attach.VirtualMachine;

public final class SelectionAttach {
    public static void main(String[] args) throws Exception {
        var vm = VirtualMachine.attach(args[0]);
        try { vm.loadAgent(args[1], args[1]); }
        finally { vm.detach(); }
    }
}
