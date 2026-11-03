// Dump decompiled functions for code addresses auto-analysis did not turn into functions.
//
// The shipped AI instinct handlers are registered by pointer (`sithAI_RegisterInstinct(name,
// &handler, ...)`), so nothing calls them directly and the default analyzer leaves them as raw
// bytes: ExportDecomp.java cannot emit them. This script disassembles each requested address,
// creates the function, decompiles it, and appends both the file and its INDEX.txt line.
//
//   M:\ghidra_12.1.4_PUBLIC\support\analyzeHeadless.bat <project> <name> ^
//       -import "G:\GameDev\bevi-knight\target\parity\retailcopy\JK.EXE" ^
//       -scriptPath tools\ghidra -postScript ExportAt.java decomp 0x50aa80 -deleteProject
//
// Arguments: output directory, then one or more hexadecimal addresses.
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressFactory;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;

import java.io.File;
import java.io.FileWriter;
import java.io.PrintWriter;

public class ExportAt extends GhidraScript {

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) {
            println("usage: ExportAt.java <output dir> <0xaddress> [0xaddress ...]");
            return;
        }
        File out = new File(args[0]);
        if (!out.exists() && !out.mkdirs()) {
            println("cannot create " + out);
            return;
        }

        DecompInterface decompiler = new DecompInterface();
        decompiler.toggleCCode(true);
        decompiler.toggleSyntaxTree(false);
        if (!decompiler.openProgram(currentProgram)) {
            println("decompiler refused: " + decompiler.getLastMessage());
            return;
        }

        AddressFactory factory = currentProgram.getAddressFactory();
        FunctionManager functions = currentProgram.getFunctionManager();

        for (int i = 1; i < args.length; i++) {
            String text = args[i].trim();
            long value = text.startsWith("0x") || text.startsWith("0X")
                    ? Long.parseLong(text.substring(2), 16)
                    : Long.parseLong(text, 16);
            Address address = factory.getDefaultAddressSpace().getAddress(value);
            Function function = functions.getFunctionAt(address);
            if (function == null) {
                // The handler is referenced but never called, so its bytes were never disassembled.
                if (getInstructionAt(address) == null) {
                    disassemble(address);
                }
                function = createFunction(address, null);
            }
            if (function == null) {
                println(String.format("%s: no function and could not create one", address));
                continue;
            }

            String hex = String.format("%08x", function.getEntryPoint().getUnsignedOffset());
            String name = function.getName().replaceAll("[^A-Za-z0-9_.-]", "_");
            String file = hex + "_" + name + ".c";
            DecompileResults results = decompiler.decompileFunction(function, 60, monitor);
            String body = results != null && results.decompileCompleted()
                    ? results.getDecompiledFunction().getC()
                    : "// decompilation failed: "
                            + (results == null ? "no result" : results.getErrorMessage());

            try (PrintWriter writer = new PrintWriter(new FileWriter(new File(out, file)))) {
                writer.println("// " + function.getName() + " @ " + function.getEntryPoint());
                writer.println("// " + function.getSignature());
                writer.println();
                writer.println(body);
            }
            try (PrintWriter index = new PrintWriter(new FileWriter(new File(out, "INDEX.txt"), true))) {
                index.println(hex + "\t" + function.getName() + "\t" + file);
            }
            println("wrote " + file);
        }
        decompiler.dispose();
    }
}
