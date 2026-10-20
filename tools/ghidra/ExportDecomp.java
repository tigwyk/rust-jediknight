// Dump every decompiled function of the imported program into a directory.
//
// Run through Ghidra's headless launcher, one script argument: the output directory.
//
//   M:\ghidra_12.1.4_PUBLIC\support\analyzeHeadless.bat <project> <name> ^
//       -import "G:\Games\Star Wars Jedi Knight - Dark Forces 2\JK.EXE" ^
//       -scriptPath tools\ghidra -postScript ExportDecomp.java decomp -deleteProject
//
// Writes `<address>_<name>.c` per procedure plus `INDEX.txt` (address, name, file), so the
// result can be grepped and read the same way the port's own sources are. This is a reading
// aid: Ghidra's text is decompiler output, not the game's source, and a function it fails on
// is written as a comment saying so rather than silently dropped.
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;

import java.io.File;
import java.io.FileWriter;
import java.io.PrintWriter;

public class ExportDecomp extends GhidraScript {

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        File out = new File(args.length > 0 ? args[0] : "decomp");
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

        int total = 0;
        int written = 0;
        int failed = 0;
        try (PrintWriter index = new PrintWriter(new FileWriter(new File(out, "INDEX.txt")))) {
            index.println("# address\tname\tfile");
            FunctionIterator functions = currentProgram.getFunctionManager().getFunctions(true);
            while (functions.hasNext() && !monitor.isCancelled()) {
                Function function = functions.next();
                total++;
                String hex = String.format("%08x", function.getEntryPoint().getUnsignedOffset());
                String name = function.getName().replaceAll("[^A-Za-z0-9_.-]", "_");
                String file = hex + "_" + name + ".c";

                DecompileResults results = decompiler.decompileFunction(function, 60, monitor);
                boolean ok = results != null && results.decompileCompleted();
                String body;
                if (ok) {
                    body = results.getDecompiledFunction().getC();
                } else {
                    failed++;
                    body = "// decompilation failed: "
                            + (results == null ? "no result" : results.getErrorMessage());
                }

                try (PrintWriter writer = new PrintWriter(new FileWriter(new File(out, file)))) {
                    writer.println("// " + function.getName() + " @ " + function.getEntryPoint());
                    writer.println("// " + function.getSignature());
                    writer.println("// callers: " + function.getCallingFunctions(monitor).size()
                            + "  callees: " + function.getCalledFunctions(monitor).size());
                    writer.println();
                    writer.println(body);
                }
                index.println(hex + "\t" + function.getName() + "\t" + file);
                written++;
                if (total % 250 == 0) {
                    println("exported " + total);
                }
            }
        }
        decompiler.dispose();
        println("exported " + written + " function(s) to " + out
                + " (" + failed + " failed to decompile, " + total + " seen)");
    }
}
