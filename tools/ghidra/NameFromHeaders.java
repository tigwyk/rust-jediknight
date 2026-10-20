// Name JK.EXE's stripped functions from a directory of headers that carry their addresses.
//
// JK.EXE ships with no symbols, so Ghidra calls everything `FUN_004cfb30`. OpenJKDF2's headers
// record the address of each engine function it reimplements, one line each:
//
//     #define sithWorld_Load_ADDR          (0x004CFB30)
//     #define sithThing_SetSector_ADDR     (0x004CD1E0)
//
// This reads those lines, renames the function at each address, and labels nothing else — a define
// that points at data, or at an address Ghidra found no function at, is counted and reported rather
// than guessed at. Renaming here (rather than only renaming output files) also makes the decompiled
// bodies readable: a call site prints `sithWorld_Load(...)` instead of `FUN_004cfb30(...)`.
//
// Run it *before* ExportDecomp.java in the same headless session:
//
//   analyzeHeadless.bat <project> <name> -import <exe> -scriptPath tools\ghidra ^
//       -postScript NameFromHeaders.java <headers dir> ^
//       -postScript ExportDecomp.java decomp -deleteProject
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSpace;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.SourceType;

import java.io.File;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public class NameFromHeaders extends GhidraScript {

    // `#define <name>_ADDR` then an address literal, with or without parentheses and with any case.
    private static final Pattern DEFINE =
            Pattern.compile("^\\s*#define\\s+([A-Za-z_][A-Za-z0-9_]*?)_ADDR\\s+\\(?\\s*0[xX]([0-9A-Fa-f]+)\\s*\\)?");

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            println("usage: NameFromHeaders.java <headers directory>");
            return;
        }
        File root = new File(args[0]);
        if (!root.isDirectory()) {
            println("not a directory: " + root);
            return;
        }

        // One address can carry several defines (aliases). The first one wins and the rest are
        // reported, because a second rename would just overwrite the first.
        Map<Long, String> wanted = new LinkedHashMap<>();
        int duplicates = 0;
        List<File> headers = new ArrayList<>();
        collect(root, headers);
        for (File header : headers) {
            for (String line : Files.readAllLines(header.toPath())) {
                Matcher match = DEFINE.matcher(line);
                if (!match.find()) {
                    continue;
                }
                long address = Long.parseLong(match.group(2), 16);
                String name = match.group(1);
                String previous = wanted.putIfAbsent(address, name);
                if (previous != null && !previous.equals(name)) {
                    duplicates++;
                }
            }
        }

        AddressSpace space = currentProgram.getAddressFactory().getDefaultAddressSpace();
        int renamed = 0;
        int notAFunction = 0;
        int rejected = 0;
        for (Map.Entry<Long, String> entry : wanted.entrySet()) {
            Address address = space.getAddress(entry.getKey());
            Function function = getFunctionAt(address);
            if (function == null) {
                notAFunction++;
                continue;
            }
            try {
                function.setName(entry.getValue(), SourceType.USER_DEFINED);
                renamed++;
            } catch (Exception invalid) {
                rejected++;
            }
        }
        println("names: " + renamed + " function(s) named from " + headers.size() + " header(s), "
                + wanted.size() + " address(es) defined; " + notAFunction
                + " had no function, " + rejected + " name(s) rejected, " + duplicates
                + " duplicate address(es)");
    }

    private void collect(File directory, List<File> out) {
        File[] entries = directory.listFiles();
        if (entries == null) {
            return;
        }
        for (File entry : entries) {
            if (entry.isDirectory()) {
                collect(entry, out);
            } else if (entry.getName().endsWith(".h")) {
                out.add(entry);
            }
        }
    }
}
