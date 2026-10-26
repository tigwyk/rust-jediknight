// bevi-knight uninstaller.
//
// Installed by the setup as bevi-knight-uninstall.exe and registered as the Apps & Features
// uninstall command. Removes the shortcuts, the per-user uninstall entry, the saved game path and
// the install folder itself; the Star Wars Jedi Knight game install and its saves are untouched.
// Compiled by packaging/build-setup.ps1 with the in-box .NET Framework compiler.

using System;
using System.Diagnostics;
using System.IO;
using System.Windows.Forms;
using Microsoft.Win32;

static class UninstallProgram
{
    [STAThread]
    static int Main(string[] args)
    {
        bool silent = false;
        foreach (string raw in args)
        {
            string low = raw.Trim().ToLowerInvariant();
            if (low == "/s" || low == "/silent") silent = true;
        }
        string installDir = Application.StartupPath;
        if (!silent)
        {
            DialogResult answer = MessageBox.Show(
                "Remove bevi-knight from this computer?\r\n\r\n" + installDir +
                "\r\n\r\nThe Star Wars Jedi Knight game install and your saves are not touched.",
                "bevi-knight uninstall", MessageBoxButtons.YesNo, MessageBoxIcon.Question);
            if (answer != DialogResult.Yes) return 0;
            Application.EnableVisualStyles();
        }

        string[] shortcuts = { StartMenuShortcut(), DesktopShortcut() };
        foreach (string link in shortcuts)
        {
            try
            {
                if (File.Exists(link)) File.Delete(link);
            }
            catch (Exception)
            {
            }
        }
        try
        {
            Registry.CurrentUser.DeleteSubKeyTree(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\bevi-knight", false);
        }
        catch (Exception)
        {
        }
        try
        {
            string configDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "bevi-knight");
            string config = Path.Combine(configDir, "game-path.txt");
            if (File.Exists(config)) File.Delete(config);
            if (Directory.Exists(configDir) && Directory.GetFileSystemEntries(configDir).Length == 0)
                Directory.Delete(configDir);
        }
        catch (Exception)
        {
        }

        if (!silent)
        {
            MessageBox.Show("bevi-knight has been removed.", "bevi-knight uninstall",
                MessageBoxButtons.OK, MessageBoxIcon.Information);
        }
        // A running executable cannot delete its own folder; hand that to cmd after we exit.
        try
        {
            ProcessStartInfo info = new ProcessStartInfo("cmd.exe",
                "/c ping -n 2 127.0.0.1 >nul & rmdir /s /q \"" + installDir + "\"");
            info.CreateNoWindow = true;
            info.UseShellExecute = false;
            Process.Start(info);
        }
        catch (Exception)
        {
        }
        return 0;
    }

    static string StartMenuShortcut()
    {
        return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "bevi-knight.lnk");
    }

    static string DesktopShortcut()
    {
        return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory), "bevi-knight.lnk");
    }
}
