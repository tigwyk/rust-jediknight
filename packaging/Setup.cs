// bevi-knight native Windows setup.
//
// Built by packaging/build-setup.ps1 with the .NET Framework compiler that ships with Windows
// (no Inno Setup, WiX, NSIS or any other installer framework). It is a normal Win32 dialog:
// it finds the retail install (GOG registry, usual folders), lets you browse or drop a folder,
// checks it, installs per-user under %LOCALAPPDATA%\Programs\bevi-knight from a payload embedded
// in this executable, writes the saved game path, creates shortcuts and an Apps & Features
// uninstall entry. No admin rights and no changes to the game install.
//
//   bevi-knight-<version>-setup.exe /S /GAMEPATH="C:\GOG Games\..." /DIR="..."   (silent)
//   bevi-knight-<version>-setup.exe /uninstall [/S]                             (uninstall)

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Reflection;
using System.Windows.Forms;
using Microsoft.Win32;

static class Program
{
    [STAThread]
    static int Main(string[] args)
    {
        bool silent = false;
        bool uninstall = false;
        bool shortcuts = true;
        bool desktop = false;
        bool launch = false;
        string gamePath = null;
        string installDir = null;
        try
        {
            Installer.Log("start: " + string.Join(" ", args) + "  runtime " + Environment.Version);
            foreach (string raw in args)
        {
            string arg = raw.Trim();
            string low = arg.ToLowerInvariant();
            if (low == "/s" || low == "/silent") silent = true;
            else if (low == "/uninstall") uninstall = true;
            else if (low == "/noshortcuts") shortcuts = false;
            else if (low == "/nodesktop") desktop = false;
            else if (low == "/desktop") desktop = true;
            else if (low == "/nolaunch") launch = false;
            else if (low == "/launch") launch = true;
            else if (low.StartsWith("/gamepath=")) gamePath = Unquote(arg.Substring(10));
            else if (low.StartsWith("/dir=")) installDir = Unquote(arg.Substring(5));
        }
        if (string.IsNullOrEmpty(installDir))
            installDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                                      "Programs", "bevi-knight");

        if (uninstall)
        {
            string uninstaller = Path.Combine(installDir, "bevi-knight-uninstall.exe");
            if (!File.Exists(uninstaller))
            {
                if (silent) Installer.Log("not installed at " + installDir);
                else MessageBox.Show("bevi-knight is not installed at " + installDir + ".", "bevi-knight setup",
                    MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return 1;
            }
            Process.Start(uninstaller, silent ? "/S" : "");
            return 0;
        }

        if (string.IsNullOrEmpty(gamePath))
            gamePath = Installer.AutoFindGame();

        if (silent)
        {
            if (!Installer.ValidGameFolder(gamePath))
            {
                Installer.Log("no retail install found; pass /GAMEPATH=<folder that contains JK.EXE>");
                return 2;
            }
            try
            {
                Installer.Install(gamePath, installDir, shortcuts, desktop, launch);
                return 0;
            }
            catch (Exception ex)
            {
                Installer.Log("install failed: " + ex);
                return 1;
            }
        }

        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new SetupForm(gamePath, installDir, shortcuts, desktop, launch));
        return 0;
        }
        catch (Exception ex)
        {
            Installer.Log("fatal: " + ex);
            if (!silent) MessageBox.Show(ex.ToString(), "bevi-knight setup", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return 1;
        }
    }

    internal static string Unquote(string value)
    {
        string v = value.Trim();
        if (v.Length >= 2 && v[0] == '"' && v[v.Length - 1] == '"')
            return v.Substring(1, v.Length - 2);
        return v;
    }
}

static class Installer
{
    internal const string ResourceName = "payload.zip";
    internal const string ExeName = "bevi-knight.exe";
    internal const string UninstallerName = "bevi-knight-uninstall.exe";
    internal static readonly string[] EngineExes = { "jk.exe", "jediknight.exe", "jediknight2.exe" };

    internal static string Version
    {
        get { return BuildInfo.Version; }
    }

    internal static string LogFile
    {
        get
        {
            string overridePath = Environment.GetEnvironmentVariable("BK_SETUP_LOG");
            if (!string.IsNullOrEmpty(overridePath)) return overridePath;
            return Path.Combine(Path.GetTempPath(), "bevi-knight-setup.log");
        }
    }

    internal static void Log(string message)
    {
        try
        {
            File.AppendAllText(LogFile, DateTime.Now.ToString("HH:mm:ss") + "  " + message + "\r\n");
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("setup log unavailable: " + ex.Message);
        }
    }

    // A retail folder is: a directory holding the engine executable and the Resource folder the
    // archives sit under. This rejects a GOG Games root that merely contains the title in its path.
    internal static bool ValidGameFolder(string dir)
    {
        if (string.IsNullOrEmpty(dir) || !Directory.Exists(dir))
            return false;
        bool engine = false;
        try
        {
            foreach (string file in Directory.GetFiles(dir))
            {
                string name = Path.GetFileName(file).ToLowerInvariant();
                foreach (string want in EngineExes)
                {
                    if (name == want) engine = true;
                }
            }
        }
        catch (Exception)
        {
            return false;
        }
        return engine && Directory.Exists(Path.Combine(dir, "Resource"));
    }

    internal static string AutoFindGame()
    {
        foreach (string path in RegistryCandidates())
        {
            if (ValidGameFolder(path)) return path;
        }
        foreach (string path in CommonFolderCandidates())
        {
            if (ValidGameFolder(path)) return path;
        }
        return null;
    }

    static List<string> RegistryCandidates()
    {
        List<string> found = new List<string>();
        RegistryHive[] hives = { RegistryHive.LocalMachine, RegistryHive.CurrentUser };
        RegistryView[] views = { RegistryView.Registry64, RegistryView.Registry32 };
        foreach (RegistryHive hive in hives)
        {
            foreach (RegistryView view in views)
            {
                try
                {
                    using (RegistryKey baseKey = RegistryKey.OpenBaseKey(hive, view))
                    using (RegistryKey games = baseKey.OpenSubKey(@"SOFTWARE\GOG.com\Games"))
                    {
                        if (games == null) continue;
                        foreach (string id in games.GetSubKeyNames())
                        {
                            try
                            {
                                using (RegistryKey game = games.OpenSubKey(id))
                                {
                                    if (game == null) continue;
                                    string path = game.GetValue("path") as string;
                                    if (!string.IsNullOrEmpty(path)) found.Add(path);
                                }
                            }
                            catch (Exception)
                            {
                            }
                        }
                    }
                }
                catch (Exception)
                {
                }
            }
        }
        return found;
    }

    static List<string> CommonFolderCandidates()
    {
        List<string> found = new List<string>();
        string[] roots = { "GOG Games", "Games", @"Program Files (x86)\GOG Galaxy\Games", @"Program Files\GOG Galaxy\Games" };
        foreach (DriveInfo drive in DriveInfo.GetDrives())
        {
            if (drive.DriveType != DriveType.Fixed) continue;
            foreach (string root in roots)
            {
                string path = Path.Combine(drive.RootDirectory.FullName, root);
                if (!Directory.Exists(path)) continue;
                try
                {
                    foreach (string child in Directory.GetDirectories(path))
                        found.Add(child);
                }
                catch (Exception)
                {
                }
            }
        }
        return found;
    }

    internal static void Install(string gamePath, string installDir, bool shortcuts, bool desktop, bool launchApp)
    {
        if (!ValidGameFolder(gamePath))
            throw new InvalidOperationException("That folder is not a Jedi Knight install (expected JK.EXE and a Resource folder).");
        Log("installing " + Version + " from " + gamePath + " to " + installDir);
        Directory.CreateDirectory(installDir);
        ExtractPayload(installDir);
        File.WriteAllText(Path.Combine(installDir, "game-path.txt"), gamePath.TrimEnd('\\') + "\r\n");
        if (shortcuts) CreateShortcuts(installDir, desktop);
        WriteUninstallEntry(installDir, gamePath);
        Log("installed");
        if (launchApp) Launch(installDir);
    }

    static void ExtractPayload(string installDir)
    {
        Assembly assembly = Assembly.GetExecutingAssembly();
        using (Stream payload = assembly.GetManifestResourceStream(ResourceName))
        {
            if (payload == null)
                throw new InvalidOperationException("this setup.exe has no embedded payload");
            string tempRoot = Path.Combine(Path.GetTempPath(), "bevi-knight-setup-" + Guid.NewGuid().ToString("N"));
            string tempZip = Path.Combine(tempRoot, "payload.zip");
            string tempDir = Path.Combine(tempRoot, "files");
            Directory.CreateDirectory(tempRoot);
            try
            {
                using (FileStream file = File.Create(tempZip))
                    payload.CopyTo(file);
                ZipFile.ExtractToDirectory(tempZip, tempDir);
                CopyFolder(tempDir, installDir);
            }
            finally
            {
                TryDeleteDirectory(tempRoot);
            }
        }
    }

    static void CopyFolder(string from, string to)
    {
        Directory.CreateDirectory(to);
        foreach (string dir in Directory.GetDirectories(from))
            CopyFolder(dir, Path.Combine(to, Path.GetFileName(dir)));
        foreach (string file in Directory.GetFiles(from))
        {
            string target = Path.Combine(to, Path.GetFileName(file));
            File.Copy(file, target, true);
        }
    }

    static void TryDeleteDirectory(string dir)
    {
        try
        {
            if (Directory.Exists(dir)) Directory.Delete(dir, true);
        }
        catch (Exception)
        {
        }
    }

    internal static string StartMenuShortcut
    {
        get
        {
            return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "bevi-knight.lnk");
        }
    }

    internal static string DesktopShortcut
    {
        get
        {
            return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory), "bevi-knight.lnk");
        }
    }

    static void CreateShortcuts(string installDir, bool desktop)
    {
        string exe = Path.Combine(installDir, ExeName);
        CreateShortcut(StartMenuShortcut, exe, installDir);
        if (desktop) CreateShortcut(DesktopShortcut, exe, installDir);
    }

    static void CreateShortcut(string linkPath, string target, string workingDir)
    {
        Type shellType = Type.GetTypeFromProgID("WScript.Shell");
        if (shellType == null) return;
        object shell = Activator.CreateInstance(shellType);
        object link = shellType.InvokeMember("CreateShortcut", BindingFlags.InvokeMethod, null, shell, new object[] { linkPath });
        Type linkType = link.GetType();
        linkType.InvokeMember("TargetPath", BindingFlags.SetProperty, null, link, new object[] { target });
        linkType.InvokeMember("WorkingDirectory", BindingFlags.SetProperty, null, link, new object[] { workingDir });
        linkType.InvokeMember("Description", BindingFlags.SetProperty, null, link, new object[] { "Star Wars Jedi Knight: Dark Forces II reconstruction" });
        linkType.InvokeMember("IconLocation", BindingFlags.SetProperty, null, link, new object[] { target + ",0" });
        linkType.InvokeMember("Save", BindingFlags.InvokeMethod, null, link, new object[0]);
    }

    static void WriteUninstallEntry(string installDir, string gamePath)
    {
        string exe = Path.Combine(installDir, UninstallerName);
        long size = 0;
        try
        {
            foreach (string file in Directory.GetFiles(installDir, "*", SearchOption.AllDirectories))
                size += new FileInfo(file).Length;
        }
        catch (Exception)
        {
        }
        using (RegistryKey key = Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\bevi-knight"))
        {
            key.SetValue("DisplayName", "bevi-knight " + Version);
            key.SetValue("DisplayVersion", Version);
            key.SetValue("DisplayIcon", Path.Combine(installDir, ExeName) + ",0");
            key.SetValue("InstallLocation", installDir);
            key.SetValue("UninstallString", "\"" + exe + "\"");
            key.SetValue("NoModify", 1, RegistryValueKind.DWord);
            key.SetValue("NoRepair", 1, RegistryValueKind.DWord);
            key.SetValue("EstimatedSize", (int)(size / 1024), RegistryValueKind.DWord);
            key.SetValue("Comments", "Game data: " + gamePath);
        }
    }

    static void Launch(string installDir)
    {
        try
        {
            ProcessStartInfo info = new ProcessStartInfo(Path.Combine(installDir, ExeName));
            info.WorkingDirectory = installDir;
            info.UseShellExecute = true;
            Process.Start(info);
        }
        catch (Exception ex)
        {
            Log("could not launch the game: " + ex.Message);
        }
    }
}

class SetupForm : Form
{
    readonly string installDir;
    readonly bool wantShortcuts;
    readonly bool wantDesktop;
    readonly bool wantLaunch;
    readonly TextBox gamePath;
    readonly Label gameStatus;
    readonly Label whereLabel;
    readonly ProgressBar progress;
    readonly Label status;
    readonly Button installButton;
    readonly Button browseButton;
    readonly CheckBox startMenuBox;
    readonly CheckBox desktopBox;
    readonly CheckBox launchBox;
    readonly Timer validationTimer;

    internal SetupForm(string detectedGame, string installDir, bool shortcuts, bool desktop, bool launch)
    {
        this.installDir = installDir;
        this.wantShortcuts = shortcuts;
        this.wantDesktop = desktop;
        this.wantLaunch = launch;

        Text = "bevi-knight " + Installer.Version + " setup";
        ClientSize = new Size(760, 560);
        FormBorderStyle = FormBorderStyle.FixedSingle;
        MaximizeBox = false;
        StartPosition = FormStartPosition.CenterScreen;
        BackColor = Color.White;
        Font = new Font("Segoe UI", 9.75f);
        AllowDrop = true;

        Panel header = new Panel();
        header.Dock = DockStyle.Top;
        header.Height = 104;
        header.BackColor = Color.FromArgb(20, 28, 46);
        Controls.Add(header);

        Label title = new Label();
        title.Text = "bevi-knight";
        title.ForeColor = Color.White;
        title.Font = new Font("Segoe UI Semibold", 22f);
        title.AutoSize = true;
        title.Location = new Point(28, 16);
        header.Controls.Add(title);

        Label subtitle = new Label();
        subtitle.Text = "A reconstruction of Star Wars Jedi Knight: Dark Forces II";
        subtitle.ForeColor = Color.FromArgb(200, 210, 230);
        subtitle.Font = new Font("Segoe UI", 10.5f);
        subtitle.AutoSize = true;
        subtitle.Location = new Point(31, 62);
        header.Controls.Add(subtitle);

        Label version = new Label();
        version.Text = "version " + Installer.Version;
        version.ForeColor = Color.FromArgb(150, 165, 195);
        version.AutoSize = true;
        version.Anchor = AnchorStyles.Top | AnchorStyles.Right;
        version.Location = new Point(ClientSize.Width - 150, 30);
        header.Controls.Add(version);

        Label step1 = new Label();
        step1.Text = "1   Find your retail game";
        step1.Font = new Font("Segoe UI Semibold", 11.5f);
        step1.AutoSize = true;
        step1.Location = new Point(30, 126);
        Controls.Add(step1);

        Label explain = new Label();
        explain.Text = "bevi-knight ships no game data. Point it at the folder that contains JK.EXE and Resource\\ " +
            "(the GOG installer's game folder). You can browse, paste a path, or drop the folder here.";
        explain.AutoSize = false;
        explain.Size = new Size(700, 44);
        explain.Location = new Point(30, 154);
        explain.ForeColor = Color.FromArgb(70, 70, 76);
        Controls.Add(explain);

        gamePath = new TextBox();
        gamePath.Location = new Point(30, 202);
        gamePath.Size = new Size(600, 28);
        gamePath.Text = detectedGame ?? "";
        gamePath.AllowDrop = true;
        Controls.Add(gamePath);

        browseButton = new Button();
        browseButton.Text = "Browse...";
        browseButton.Location = new Point(640, 200);
        browseButton.Size = new Size(90, 30);
        browseButton.Click += BrowseClicked;
        Controls.Add(browseButton);

        gameStatus = new Label();
        gameStatus.AutoSize = true;
        gameStatus.Location = new Point(30, 236);
        Controls.Add(gameStatus);

        Label step2 = new Label();
        step2.Text = "2   Options";
        step2.Font = new Font("Segoe UI Semibold", 11.5f);
        step2.AutoSize = true;
        step2.Location = new Point(30, 274);
        Controls.Add(step2);

        startMenuBox = new CheckBox();
        startMenuBox.Text = "Start Menu shortcut";
        startMenuBox.Checked = shortcuts;
        startMenuBox.AutoSize = true;
        startMenuBox.Location = new Point(30, 304);
        Controls.Add(startMenuBox);

        desktopBox = new CheckBox();
        desktopBox.Text = "Desktop shortcut";
        desktopBox.Checked = this.wantDesktop;
        desktopBox.AutoSize = true;
        desktopBox.Location = new Point(250, 304);
        Controls.Add(desktopBox);

        launchBox = new CheckBox();
        launchBox.Text = "Launch bevi-knight when setup finishes";
        launchBox.Checked = this.wantLaunch;
        launchBox.AutoSize = true;
        launchBox.Location = new Point(30, 332);
        Controls.Add(launchBox);

        whereLabel = new Label();
        whereLabel.AutoSize = true;
        whereLabel.ForeColor = Color.FromArgb(90, 90, 96);
        whereLabel.Location = new Point(30, 372);
        whereLabel.Text = "Installs to: " + this.installDir;
        Controls.Add(whereLabel);

        progress = new ProgressBar();
        progress.Location = new Point(30, 402);
        progress.Size = new Size(700, 16);
        progress.Style = ProgressBarStyle.Marquee;
        progress.MarqueeAnimationSpeed = 30;
        progress.Visible = false;
        Controls.Add(progress);

        status = new Label();
        status.AutoSize = true;
        status.Location = new Point(30, 426);
        Controls.Add(status);

        Panel footer = new Panel();
        footer.Dock = DockStyle.Bottom;
        footer.Height = 64;
        footer.BackColor = Color.FromArgb(246, 246, 248);
        Controls.Add(footer);

        installButton = new Button();
        installButton.Text = "Install";
        installButton.Size = new Size(120, 32);
        installButton.Location = new Point(ClientSize.Width - 270, 16);
        installButton.Click += InstallClicked;
        footer.Controls.Add(installButton);

        Button cancelButton = new Button();
        cancelButton.Text = "Cancel";
        cancelButton.Size = new Size(120, 32);
        cancelButton.Location = new Point(ClientSize.Width - 140, 16);
        cancelButton.Click += delegate { Close(); };
        footer.Controls.Add(cancelButton);
        CancelButton = cancelButton;
        AcceptButton = installButton;

        Label note = new Label();
        note.Text = "Per-user install: no admin rights, no changes to the game folder.";
        note.AutoSize = true;
        note.ForeColor = Color.FromArgb(120, 120, 126);
        note.Location = new Point(30, 16);
        footer.Controls.Add(note);

        DragEnter += DragEntered;
        DragDrop += Dropped;
        gamePath.DragEnter += DragEntered;
        gamePath.DragDrop += Dropped;

        validationTimer = new Timer();
        validationTimer.Interval = 200;
        validationTimer.Tick += delegate { ValidatePath(); };
        gamePath.TextChanged += delegate { validationTimer.Stop(); validationTimer.Start(); };
        Shown += delegate { ValidatePath(); };
    }

    void BrowseClicked(object sender, EventArgs e)
    {
        using (FolderBrowserDialog dialog = new FolderBrowserDialog())
        {
            dialog.Description = "Select the Star Wars Jedi Knight: Dark Forces II folder (the one with JK.EXE)";
            dialog.ShowNewFolderButton = false;
            string current = gamePath.Text.Trim();
            if (Directory.Exists(current)) dialog.SelectedPath = current;
            if (dialog.ShowDialog(this) == DialogResult.OK)
                gamePath.Text = dialog.SelectedPath;
        }
    }

    static string DroppedFolder(DragEventArgs e)
    {
        if ((e.Data.GetDataPresent(DataFormats.FileDrop)) == false) return null;
        string[] entries = e.Data.GetData(DataFormats.FileDrop) as string[];
        if (entries == null || entries.Length != 1) return null;
        if (Directory.Exists(entries[0])) return entries[0];
        return null;
    }

    void DragEntered(object sender, DragEventArgs e)
    {
        e.Effect = DroppedFolder(e) == null ? DragDropEffects.None : DragDropEffects.Copy;
    }

    void Dropped(object sender, DragEventArgs e)
    {
        string folder = DroppedFolder(e);
        if (folder != null) gamePath.Text = folder;
    }

    void ValidatePath()
    {
        validationTimer.Stop();
        string path = gamePath.Text.Trim();
        bool valid = Installer.ValidGameFolder(path);
        if (path.Length == 0)
        {
            gameStatus.Text = "Choose the folder that holds JK.EXE.";
            gameStatus.ForeColor = Color.FromArgb(150, 90, 30);
        }
        else if (valid)
        {
            gameStatus.Text = "Retail install found.";
            gameStatus.ForeColor = Color.FromArgb(20, 120, 50);
        }
        else
        {
            gameStatus.Text = "That folder has no JK.EXE or no Resource folder.";
            gameStatus.ForeColor = Color.FromArgb(170, 40, 40);
        }
        installButton.Enabled = valid;
    }

    void InstallClicked(object sender, EventArgs e)
    {
        string path = gamePath.Text.Trim();
        if (!Installer.ValidGameFolder(path))
        {
            MessageBox.Show(this, "Choose the retail game folder first.", "bevi-knight setup",
                MessageBoxButtons.OK, MessageBoxIcon.Warning);
            return;
        }
        SetBusy(true, "Installing...");
        try
        {
            Installer.Install(path, installDir, startMenuBox.Checked, desktopBox.Checked, launchBox.Checked);
            SetBusy(false, "Installed.");
            MessageBox.Show(this,
                "bevi-knight is installed.\r\n\r\n" + installDir +
                "\r\n\r\nStart it from the Start Menu (or the shortcut you chose). The Star Wars " +
                "Jedi Knight game folder was not modified.",
                "bevi-knight setup", MessageBoxButtons.OK, MessageBoxIcon.Information);
            Close();
        }
        catch (Exception ex)
        {
            SetBusy(false, "Install failed.");
            MessageBox.Show(this, ex.Message, "bevi-knight setup", MessageBoxButtons.OK, MessageBoxIcon.Error);
        }
    }

    void SetBusy(bool busy, string message)
    {
        progress.Visible = busy;
        status.Text = message;
        gamePath.Enabled = !busy;
        browseButton.Enabled = !busy;
        startMenuBox.Enabled = !busy;
        desktopBox.Enabled = !busy;
        launchBox.Enabled = !busy;
        installButton.Enabled = !busy && Installer.ValidGameFolder(gamePath.Text.Trim());
        Cursor = busy ? Cursors.WaitCursor : Cursors.Default;
        Application.DoEvents();
    }
}
