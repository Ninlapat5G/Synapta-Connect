// ตัวติดตั้งแบบหน้าต่างของ Synapta Connect (Windows x64)
//
// ห่อ synapta-connect.exe ไว้ข้างใน แล้วเรียก `synapta-connect.exe install` ด้วยสิทธิ์ผู้ดูแล — ตรรกะติดตั้งจริง
// (service, ACL, firewall, ถอนเมื่อพัง) อยู่ใน agent ที่เดียว ไฟล์นี้ทำแค่หน้าจอ ตรวจไฟล์ที่ฝัง และส่งต่อคำสั่ง
// เขียนด้วย C# 5 + WinForms บน .NET Framework 4.x ที่ Windows Server 2016 มีมาแล้ว (build ด้วย csc ของระบบ)

using System;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;
using System.Threading.Tasks;
using System.Windows.Forms;

internal static class Setup
{
    private const int ExitOk = 0;
    private const int ExitBadArguments = 2;
    private const int ExitFailed = 1;

    [STAThread]
    private static int Main(string[] args)
    {
        if (args.Length == 1 && args[0] == "--verify-payload") return VerifyPayload() ? ExitOk : ExitFailed;
        // โหมดที่หน้าต่างเรียกตัวเองซ้ำแบบยกสิทธิ์ (runas)
        if (args.Length == 5 && args[0] == "--install") return Install(args);
        if (args.Length >= 1 && args[0] == "--uninstall") return Uninstall(args.Length == 2 && args[1] == "--purge");
        if (args.Length == 1 && args[0] == "--upgrade") return Upgrade();

        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Theme.LoadFonts();
        if (args.Length == 3 && args[0] == "--preview") return Preview(args[1], args[2]);
        Application.Run(new SetupForm());
        return ExitOk;
    }

    // ---------- ไฟล์ที่ฝังไว้ ----------

    internal static Stream Resource(string name)
    {
        Stream stream = Assembly.GetExecutingAssembly().GetManifestResourceStream(name);
        if (stream == null) throw new InvalidOperationException("ตัวติดตั้งไม่มีไฟล์ " + name);
        return stream;
    }

    internal static string ResourceText(string name)
    {
        using (var reader = new StreamReader(Resource(name), Encoding.UTF8)) return reader.ReadToEnd().Trim();
    }

    /// agent ที่ฝังต้องตรงกับ hash ที่ build ใส่ไว้ — กันไฟล์ setup เสียระหว่างดาวน์โหลด/คัดลอก
    private static bool VerifyPayload()
    {
        try
        {
            using (var source = Resource("agent.exe"))
            using (var sha = SHA256.Create())
            {
                string actual = BitConverter.ToString(sha.ComputeHash(source)).Replace("-", "").ToLowerInvariant();
                return actual == ResourceText("agent.sha256");
            }
        }
        catch (Exception) { return false; }
    }

    internal static string InstalledExe
    {
        get { return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "SynaptaConnect", "synapta-connect.exe"); }
    }

    // ---------- ส่วนที่รันด้วยสิทธิ์ผู้ดูแล ----------

    private static int Install(string[] args)
    {
        int api, discovery, panel;
        if (!int.TryParse(args[1], out api) || !int.TryParse(args[2], out discovery) || !int.TryParse(args[3], out panel)
            || !ValidPort(api) || !ValidPort(discovery) || !ValidPort(panel) || api == panel)
            return ExitBadArguments;
        string owner = args[4].Replace("\"", "");
        string arguments = "install --port " + api + " --discovery-port " + discovery + " --local-port " + panel + " --owner \"" + owner + "\"";
        int code = RunAgent(arguments, "ติดตั้งไม่สำเร็จ");
        if (code == ExitOk) SaveNotices();
        return code;
    }

    /// แทนโปรแกรมที่ติดตั้งด้วยรุ่นที่ฝังมา — agent คง service, การจับคู่ และค่าตั้งไว้ (และกู้รุ่นเดิมเองถ้ารุ่นใหม่ไม่ขึ้น)
    private static int Upgrade()
    {
        int code = RunAgent("upgrade", "อัปเดตไม่สำเร็จ");
        if (code == ExitOk) SaveNotices();
        return code;
    }

    private static int Uninstall(bool purge)
    {
        if (!File.Exists(InstalledExe)) return ExitOk;
        return RunAgent(purge ? "uninstall" : "uninstall --keep-data", "ถอนการติดตั้งไม่สำเร็จ");
    }

    /// รันคำสั่งด้วย agent ที่ฝังมา (ไม่ใช่ตัวใน Program Files)
    ///
    /// กับดัก: ถ้าใช้ตัวที่ติดตั้งสั่ง uninstall มันลบ exe ตัวเองไม่ได้ จึงตั้งเวลาให้ลบตามหลังไม่กี่วินาที
    /// หน้าต่างที่ตรวจไฟล์ทันทีจะเห็นว่ายังอยู่แล้วรายงานว่าล้มทั้งที่สำเร็จ (เคยเกิดจริงบน Server 2016)
    /// แตกลงโฟลเดอร์ที่มีแค่ SYSTEM/Administrators เขียนได้ — ผู้ใช้ทั่วไปสลับไฟล์ก่อนรันด้วยสิทธิ์ผู้ดูแลไม่ได้
    private static int RunAgent(string arguments, string failureTitle)
    {
        if (!VerifyPayload()) return ExitBadArguments;
        var security = new DirectorySecurity();
        security.SetAccessRuleProtection(true, false);
        foreach (var sid in new[] { WellKnownSidType.BuiltinAdministratorsSid, WellKnownSidType.LocalSystemSid })
            security.AddAccessRule(new FileSystemAccessRule(new SecurityIdentifier(sid, null), FileSystemRights.FullControl,
                InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit, PropagationFlags.None, AccessControlType.Allow));
        string directory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData),
            "SynaptaConnectSetup-" + Guid.NewGuid().ToString("N"));
        try
        {
            Directory.CreateDirectory(directory, security);
            string exe = Path.Combine(directory, "synapta-connect.exe");
            using (var source = Resource("agent.exe"))
            using (var output = new FileStream(exe, FileMode.CreateNew, FileAccess.Write, FileShare.None)) source.CopyTo(output);
            string log;
            int code = RunHidden(exe, arguments, out log);
            if (code != 0) ShowError(failureTitle, log);
            return code;
        }
        catch (Exception error)
        {
            ShowError(failureTitle, error.Message);
            return ExitFailed;
        }
        finally
        {
            try { if (Directory.Exists(directory)) Directory.Delete(directory, true); }
            catch (Exception) { }
        }
    }

    private static void SaveNotices()
    {
        try
        {
            string notice = Path.Combine(Path.GetDirectoryName(InstalledExe), "THIRD_PARTY_NOTICES.txt");
            using (var source = Resource("notices.txt"))
            using (var output = new FileStream(notice, FileMode.Create, FileAccess.Write, FileShare.Read)) source.CopyTo(output);
        }
        catch (Exception error) { ShowError("บันทึกเอกสารสิทธิ์ไม่สำเร็จ", "ติดตั้งสำเร็จแล้ว แต่คัดลอก THIRD_PARTY_NOTICES.txt ไม่ได้: " + error.Message); }
    }

    /// รุ่นของตัวที่ติดตั้ง (`synapta-connect --version`) — อ่านไม่ได้ = null
    internal static Version InstalledVersion()
    {
        try
        {
            string log;
            if (RunHidden(InstalledExe, "--version", out log) != 0) return null;
            var match = System.Text.RegularExpressions.Regex.Match(log, @"(\d+)\.(\d+)\.(\d+)");
            return match.Success ? new Version(match.Value) : null;
        }
        catch (Exception) { return null; }
    }

    private static bool ValidPort(int port) { return port >= 1 && port <= 65535; }

    internal static int RunHidden(string exe, string arguments, out string log)
    {
        var start = new ProcessStartInfo(exe, arguments)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };
        using (var process = Process.Start(start))
        {
            var stdout = process.StandardOutput.ReadToEndAsync();
            var stderr = process.StandardError.ReadToEndAsync();
            process.WaitForExit();
            Task.WaitAll(stdout, stderr);
            log = (stdout.Result + "\n" + stderr.Result).Trim();
            return process.ExitCode;
        }
    }

    private static void ShowError(string title, string detail)
    {
        MessageBox.Show(detail, title, MessageBoxButtons.OK, MessageBoxIcon.Error);
    }

    /// ภาพหน้าจอทุกหน้าสำหรับตรวจดีไซน์ตอน build (`--preview <หน้า> <ไฟล์.png>`)
    private static int Preview(string page, string file)
    {
        try { return PreviewPage(page, file); }
        catch (Exception error)
        {
            File.WriteAllText(file + ".error.txt", error.ToString());
            return ExitFailed;
        }
    }

    private static int PreviewPage(string page, string file)
    {
        using (var form = new SetupForm())
        {
            form.ShowInTaskbar = false;
            form.StartPosition = FormStartPosition.Manual;
            form.Location = new Point(-32000, -32000);
            form.Show();
            form.ShowPreviewPage(page);
            Application.DoEvents();
            using (var bitmap = new Bitmap(form.Width, form.Height))
            {
                form.DrawToBitmap(bitmap, new Rectangle(Point.Empty, bitmap.Size));
                bitmap.Save(file, System.Drawing.Imaging.ImageFormat.Png);
            }
        }
        return ExitOk;
    }
}

/// สีและฟอนต์ชุดเดียวกับหน้า panel (frontend/src/style.css)
internal static class Theme
{
    public static readonly Color Canvas = Color.FromArgb(243, 245, 250);
    public static readonly Color Panel = Color.White;
    public static readonly Color Inside = Color.FromArgb(245, 246, 251);
    public static readonly Color Ink = Color.FromArgb(23, 34, 61);
    public static readonly Color Muted = Color.FromArgb(86, 99, 129);
    public static readonly Color Line = Color.FromArgb(219, 225, 238);
    public static readonly Color Accent = Color.FromArgb(68, 56, 191);
    public static readonly Color AccentSoft = Color.FromArgb(238, 235, 255);
    public static readonly Color Good = Color.FromArgb(8, 118, 83);
    public static readonly Color GoodSoft = Color.FromArgb(224, 245, 235);
    public static readonly Color Danger = Color.FromArgb(169, 34, 56);
    public static readonly Color Night = Color.FromArgb(12, 18, 34);

    private static FontFamily body, bodyBold, display;
    private static readonly PrivateFontCollection[] collections = new PrivateFontCollection[3];

    [DllImport("gdi32.dll")]
    private static extern IntPtr AddFontMemResourceEx(IntPtr font, uint length, IntPtr reserved, ref uint count);

    /// Anuphan และ Chakra Petch (Google Fonts, OFL) ที่ฝังมา — โหลดไม่ได้ก็ใช้ฟอนต์ระบบแทน
    public static void LoadFonts()
    {
        body = Load(0, "Anuphan-Regular.ttf");
        bodyBold = Load(1, "Anuphan-SemiBold.ttf");
        display = Load(2, "ChakraPetch-SemiBold.ttf");
    }

    private static FontFamily Load(int slot, string name)
    {
        try
        {
            byte[] data;
            using (var stream = Setup.Resource(name))
            using (var copy = new MemoryStream()) { stream.CopyTo(copy); data = copy.ToArray(); }
            // กับดัก: PrivateFontCollection อย่างเดียวใช้ได้แค่กับ GDI+ (DrawString) แต่ Label/Button วาดด้วย GDI
            // (TextRenderer) จึงต้องลงทะเบียนกับ GDI ด้วย AddFontMemResourceEx ไม่งั้นตัวควบคุมจะกลับไปใช้ฟอนต์ระบบเงียบ ๆ
            // หน่วยความจำต้องอยู่ตลอดอายุโปรเซส จึงจองแบบ unmanaged และไม่คืน
            IntPtr memory = Marshal.AllocCoTaskMem(data.Length);
            Marshal.Copy(data, 0, memory, data.Length);
            uint count = 0;
            AddFontMemResourceEx(memory, (uint)data.Length, IntPtr.Zero, ref count);
            collections[slot] = new PrivateFontCollection();
            collections[slot].AddMemoryFont(memory, data.Length);
            return collections[slot].Families[0];
        }
        catch (Exception) { return null; }
    }

    public static Font Body(float size) { return Make(body, size, FontStyle.Regular); }
    public static Font Bold(float size) { return bodyBold != null ? Make(bodyBold, size, FontStyle.Regular) : Make(body, size, FontStyle.Bold); }
    public static Font Display(float size) { return display != null ? Make(display, size, FontStyle.Regular) : Bold(size); }

    private static Font Make(FontFamily family, float size, FontStyle style)
    {
        if (family != null) return new Font(family, size, style, GraphicsUnit.Point);
        return new Font("Leelawadee UI", size, style, GraphicsUnit.Point);
    }

    public static GraphicsPath Rounded(RectangleF bounds, float radius)
    {
        float d = radius * 2;
        var path = new GraphicsPath();
        path.AddArc(bounds.X, bounds.Y, d, d, 180, 90);
        path.AddArc(bounds.Right - d, bounds.Y, d, d, 270, 90);
        path.AddArc(bounds.Right - d, bounds.Bottom - d, d, d, 0, 90);
        path.AddArc(bounds.X, bounds.Bottom - d, d, d, 90, 90);
        path.CloseFigure();
        return path;
    }
}

/// ปุ่มมุมมน: แบบหลัก (พื้นม่วง) หรือแบบรอง (ขอบ)
internal sealed class PillButton : Button
{
    public bool Primary = true;
    public Color Tone = Theme.Accent;
    private bool hover;

    public PillButton()
    {
        SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.SupportsTransparentBackColor, true);
        FlatStyle = FlatStyle.Flat;
        FlatAppearance.BorderSize = 0;
        BackColor = Color.Transparent;
        Cursor = Cursors.Hand;
        Font = Theme.Bold(10.5f);
        Height = 46;
    }

    protected override void OnMouseEnter(EventArgs e) { hover = true; Invalidate(); base.OnMouseEnter(e); }
    protected override void OnMouseLeave(EventArgs e) { hover = false; Invalidate(); base.OnMouseLeave(e); }

    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        g.Clear(Parent != null ? Parent.BackColor : Theme.Panel);
        g.SmoothingMode = SmoothingMode.AntiAlias;
        var bounds = new RectangleF(0.5f, 0.5f, Width - 1.5f, Height - 1.5f);
        Color fill, text;
        if (!Enabled) { fill = Theme.Line; text = Theme.Muted; }
        else if (Primary) { fill = hover ? ControlPaint.Dark(Tone, 0.08f) : Tone; text = Color.White; }
        else { fill = hover ? Theme.Inside : Theme.Panel; text = Tone; }
        using (var path = Theme.Rounded(bounds, Height / 2f - 2))
        {
            using (var brush = new SolidBrush(fill)) g.FillPath(brush, path);
            if (!Primary && Enabled) using (var pen = new Pen(Theme.Line, 1.2f)) g.DrawPath(pen, path);
        }
        TextRenderer.DrawText(g, Text, Font, ClientRectangle, text, TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter | TextFormatFlags.SingleLine);
        if (Focused && ShowFocusCues)
            using (var pen = new Pen(Color.FromArgb(120, Tone), 2f) { DashStyle = DashStyle.Dot })
            using (var path = Theme.Rounded(new RectangleF(3, 3, Width - 7, Height - 7), Height / 2f - 5)) g.DrawPath(pen, path);
    }
}

/// แถบซ้าย: ภาพจาก panel + โลโก้ + ชื่อรุ่น
internal sealed class BrandPanel : Panel
{
    private readonly Image artwork, logo;
    private readonly string version;

    public BrandPanel(string version)
    {
        this.version = version;
        DoubleBuffered = true;
        using (var stream = Setup.Resource("hero.jpg")) artwork = new Bitmap(Image.FromStream(stream));
        using (var stream = Setup.Resource("logo.png")) logo = new Bitmap(Image.FromStream(stream));
    }

    protected override void OnResize(EventArgs e) { base.OnResize(e); Invalidate(); }

    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        g.InterpolationMode = InterpolationMode.HighQualityBicubic;
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;

        // ภาพเต็มแถบแบบ cover แล้วไล่เงาสีกลางคืนให้อ่านตัวอักษรออก
        float scale = Math.Max((float)Width / artwork.Width, (float)Height / artwork.Height);
        float w = artwork.Width * scale, h = artwork.Height * scale;
        g.DrawImage(artwork, (Width - w) / 2f, (Height - h) / 2f, w, h);
        using (var shade = new LinearGradientBrush(ClientRectangle, Color.FromArgb(215, Theme.Night), Color.FromArgb(250, Theme.Night), 90f))
            g.FillRectangle(shade, ClientRectangle);

        int size = Math.Min(Width - 96, 168);
        var logoBox = new RectangleF((Width - size) / 2f, 54, size, size);
        using (var glow = new SolidBrush(Color.FromArgb(40, 140, 120, 255)))
        using (var ring = Theme.Rounded(RectangleF.Inflate(logoBox, 10, 10), 34)) g.FillPath(glow, ring);
        using (var clip = Theme.Rounded(logoBox, 28))
        {
            g.SetClip(clip);
            g.DrawImage(logo, logoBox);
            g.ResetClip();
        }

        float y = logoBox.Bottom + 30;
        using (var title = Theme.Display(17))
        using (var text = Theme.Body(10.5f))
        using (var small = Theme.Body(9))
        using (var light = new SolidBrush(Color.FromArgb(214, 222, 255)))
        using (var faint = new SolidBrush(Color.FromArgb(150, 164, 200)))
        using (var center = new StringFormat { Alignment = StringAlignment.Center })
        {
            g.DrawString("Synapta Connect", title, Brushes.White, new RectangleF(16, y, Width - 32, 40), center);
            g.DrawString("ให้ Synapta Code เชื่อมต่อ\nและดูแลเครื่องนี้ได้อย่างปลอดภัย", text, light, new RectangleF(24, y + 44, Width - 48, 60), center);
            g.DrawString("v" + version + "  ·  Windows x64", small, faint, new RectangleF(16, Height - 42, Width - 32, 24), center);
        }
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing) { artwork.Dispose(); logo.Dispose(); }
        base.Dispose(disposing);
    }
}

internal sealed class SetupForm : Form
{
    private readonly NumericUpDown api = Port(52401), discovery = Port(52400), panel = Port(52402);
    private readonly Panel content = new Panel { Dock = DockStyle.Fill, BackColor = Theme.Panel, Padding = new Padding(44, 40, 44, 32) };
    private readonly Label status = new Label { AutoSize = false, Dock = DockStyle.Fill, ForeColor = Theme.Danger, TextAlign = ContentAlignment.MiddleLeft };
    private readonly CheckBox purge = new CheckBox { AutoSize = true, Text = "ลบข้อมูลการจับคู่และค่าตั้งด้วย", ForeColor = Theme.Muted };
    private readonly Version embedded;
    private Version installed;

    public SetupForm()
    {
        string version = Setup.ResourceText("version.txt");
        embedded = new Version(version);
        Text = "ติดตั้ง Synapta Connect";
        Font = Theme.Body(10);
        AutoScaleMode = AutoScaleMode.Dpi;
        ClientSize = new Size(880, 620);
        FormBorderStyle = FormBorderStyle.FixedSingle;
        MaximizeBox = false;
        MinimizeBox = true;
        StartPosition = FormStartPosition.CenterScreen;
        BackColor = Theme.Panel;
        using (var stream = Setup.Resource("icon.ico")) Icon = new Icon(stream);

        Controls.Add(content);
        Controls.Add(new BrandPanel(version) { Dock = DockStyle.Left, Width = 300 });
        purge.Font = Theme.Body(9.5f);

        if (File.Exists(Setup.InstalledExe))
        {
            installed = Setup.InstalledVersion();
            ShowInstalled();
        }
        else ShowSetup();
    }

    internal void ShowPreviewPage(string page)
    {
        if (page == "progress") ShowProgress("กำลังติดตั้ง", "สร้าง service ตั้งสิทธิ์โฟลเดอร์ และเปิดไฟร์วอลล์เฉพาะเครือข่ายส่วนตัว");
        else if (page == "done") ShowDone(false);
        else if (page == "upgraded") ShowDone(true);
        else if (page == "installed") ShowInstalled();
        else if (page == "upgrade") { installed = new Version(0, 3, 2); ShowInstalled(); }
        else ShowSetup();
    }

    // ---------- หน้า ----------

    /// วางชิ้นส่วนเรียงลงล่าง แถวสุดท้ายยืดเต็มเพื่อดันปุ่มลงก้น
    /// ล้างหน้าเดิม — ทุก Show* เรียกก่อนสร้างชิ้นส่วนของหน้าใหม่
    private void ClearPage()
    {
        // ช่องพอร์ต/ข้อความสถานะใช้ข้ามหน้า (ค่าต้องอยู่) — ถอดออกก่อน dispose หน้าเก่า ไม่งั้นโดน dispose ตามไปด้วย
        foreach (Control shared in new Control[] { api, discovery, panel, status, purge })
            if (shared.Parent != null) shared.Parent.Controls.Remove(shared);
        foreach (Control old in content.Controls) old.Dispose();
        content.Controls.Clear();
    }

    private TableLayoutPanel NewPage(params Control[] rows)
    {
        content.SuspendLayout();
        var page = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = rows.Length, BackColor = Theme.Panel };
        page.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
        for (int i = 0; i < rows.Length; i++)
        {
            page.RowStyles.Add(rows[i] == null ? new RowStyle(SizeType.Percent, 100) : new RowStyle(SizeType.AutoSize));
            if (rows[i] != null) page.Controls.Add(rows[i], 0, i);
        }
        content.Controls.Add(page);
        content.ResumeLayout();
        return page;
    }

    private static Label Eyebrow(string text)
    {
        return new Label { Text = text.ToUpperInvariant(), AutoSize = true, Font = Theme.Display(9), ForeColor = Theme.Accent, Margin = new Padding(0, 0, 0, 6) };
    }

    private static Label Heading(string text)
    {
        return new Label { Text = text, AutoSize = true, Font = Theme.Display(20), ForeColor = Theme.Ink, Margin = new Padding(0, 0, 0, 6) };
    }

    private static Label Paragraph(string text)
    {
        return new Label { Text = text, AutoSize = true, MaximumSize = new Size(470, 0), Font = Theme.Body(10.5f), ForeColor = Theme.Muted, Margin = new Padding(0, 0, 0, 18) };
    }

    private void ShowSetup()
    {
        ClearPage();
        var ports = new TableLayoutPanel { AutoSize = true, Dock = DockStyle.Top, ColumnCount = 1, Margin = new Padding(0) };
        ports.Controls.Add(PortRow("Agent API", "ให้ Synapta Code เชื่อมต่อเข้ามา · TCP", api));
        ports.Controls.Add(PortRow("ค้นหาอุปกรณ์", "ให้หาเครื่องนี้เจอในเครือข่าย · UDP", discovery));
        ports.Controls.Add(PortRow("หน้าจัดการ", "เปิด panel บนเครื่องนี้ · TCP", panel));

        var install = new PillButton { Text = "ติดตั้ง Synapta Connect", Dock = DockStyle.Fill, Margin = new Padding(0, 4, 0, 0) };
        install.Click += StartInstall;
        ResetStatus();
        var note = new Label { Text = "Windows จะขอสิทธิ์ผู้ดูแลก่อนเริ่มติดตั้ง", AutoSize = true, ForeColor = Theme.Muted, Font = Theme.Body(9), Anchor = AnchorStyles.None, Margin = new Padding(0, 10, 0, 0) };
        NewPage(Eyebrow("ขั้นตอนที่ 1 จาก 2"), Heading("ตั้งค่าเครื่องปลายทาง"),
            Paragraph("ใช้ค่าเริ่มต้นได้เลย หรือเปลี่ยนพอร์ตให้เข้ากับเครือข่ายของคุณ ภายหลังเปลี่ยนได้ในหน้าเครือข่ายของ panel"),
            ports, null, status, install, note);
        AcceptButton = install;
    }

    private Control PortRow(string title, string description, NumericUpDown input)
    {
        var row = new TableLayoutPanel { ColumnCount = 2, RowCount = 2, Dock = DockStyle.Top, AutoSize = true, BackColor = Theme.Inside, Padding = new Padding(16, 8, 12, 8), Margin = new Padding(0, 0, 0, 8) };
        row.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
        row.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 112));
        row.Controls.Add(new Label { Text = title, AutoSize = true, Font = Theme.Bold(10.5f), ForeColor = Theme.Ink, Margin = new Padding(0) }, 0, 0);
        row.Controls.Add(new Label { Text = description, AutoSize = true, Font = Theme.Body(9), ForeColor = Theme.Muted, Margin = new Padding(0, 2, 0, 0) }, 0, 1);
        input.AccessibleName = title;
        input.Anchor = AnchorStyles.Right;
        row.Controls.Add(input, 1, 0);
        row.SetRowSpan(input, 2);
        return row;
    }

    private void ShowProgress(string title, string detail)
    {
        ClearPage();
        var bar = new ProgressBar { Style = ProgressBarStyle.Marquee, MarqueeAnimationSpeed = 28, Dock = DockStyle.Top, Height = 8, Margin = new Padding(0, 8, 0, 0) };
        NewPage(Eyebrow("ขั้นตอนที่ 2 จาก 2"), Heading(title), Paragraph(detail), bar, null);
    }

    private void ShowDone(bool upgraded)
    {
        ClearPage();
        var open = new PillButton { Text = "เปิดหน้าจัดการ", Dock = DockStyle.Fill, Margin = new Padding(0, 0, 0, 10) };
        open.Click += delegate { OpenPanel(); };
        var close = new PillButton { Text = "ปิด", Primary = false, Dock = DockStyle.Fill, Margin = new Padding(0) };
        close.Click += delegate { Close(); };
        ResetStatus();
        if (upgraded)
            NewPage(Badge("อัปเดตเรียบร้อย", Theme.Good, Theme.GoodSoft), Heading("ใช้รุ่น v" + embedded + " แล้ว"),
                Paragraph("การจับคู่ ค่าตั้ง และสิทธิ์ที่เปิดไว้ยังอยู่เหมือนเดิม Synapta Code เชื่อมต่อต่อได้เลยโดยไม่ต้องจับคู่ใหม่"),
                null, status, open, close);
        else
            NewPage(Badge("ติดตั้งเรียบร้อย", Theme.Good, Theme.GoodSoft), Heading("เครื่องนี้พร้อมเชื่อมต่อแล้ว"),
                Paragraph("Synapta Connect ทำงานเป็น service เบื้องหลังและเริ่มเองทุกครั้งที่เปิดเครื่อง เปิดหน้าจัดการเพื่อสร้างรหัสจับคู่ให้ Synapta Code"),
                Summary(), null, status, open, close);
        AcceptButton = open;
    }

    private void ShowInstalled()
    {
        ClearPage();
        bool outdated = installed != null && installed < embedded;
        var open = new PillButton { Text = "เปิดหน้าจัดการ", Primary = !outdated, Dock = DockStyle.Fill, Margin = new Padding(0, 0, 0, 10) };
        open.Click += delegate { OpenPanel(); };
        var remove = new PillButton { Text = "ถอนการติดตั้ง", Primary = false, Tone = Theme.Danger, Dock = DockStyle.Fill, Margin = new Padding(0) };
        remove.Click += StartUninstall;
        purge.Margin = new Padding(0, 0, 0, 10);
        ResetStatus();
        string current = installed == null ? "" : "v" + installed;
        if (outdated)
        {
            var upgrade = new PillButton { Text = "อัปเดตเป็น v" + embedded, Dock = DockStyle.Fill, Margin = new Padding(0, 0, 0, 10) };
            upgrade.Click += StartUpgrade;
            NewPage(Badge("มีรุ่นใหม่", Theme.Accent, Theme.AccentSoft), Heading("อัปเดตจาก " + current + " เป็น v" + embedded),
                Paragraph("อัปเดตทับได้เลยโดยไม่ต้องถอน การจับคู่ ค่าตั้ง และสิทธิ์ที่เปิดไว้จะอยู่ครบ ถ้ารุ่นใหม่เริ่มไม่ขึ้น ระบบกลับไปใช้รุ่นเดิมให้เอง"),
                null, status, upgrade, open, purge, remove);
            AcceptButton = upgrade;
            return;
        }
        string detail = installed != null && installed > embedded
            ? "เครื่องนี้มี " + current + " ซึ่งใหม่กว่าตัวติดตั้งนี้ (v" + embedded + ") ใช้ต่อได้เลย"
            : "รุ่น " + (current == "" ? "v" + embedded : current) + " ทำงานอยู่ ถ้าต้องการลงใหม่ด้วยพอร์ตอื่นให้ถอนการติดตั้งก่อน";
        NewPage(Badge("ติดตั้งอยู่แล้ว", Theme.Accent, Theme.AccentSoft), Heading("ติดตั้งบนเครื่องนี้แล้ว"),
            Paragraph(detail), null, status, purge, open, remove);
        AcceptButton = open;
    }

    private void ResetStatus()
    {
        status.Text = "";
        status.ForeColor = Theme.Danger;
        status.Height = 30;
    }

    private static Control Badge(string text, Color tone, Color soft)
    {
        return new Label { Text = "●  " + text, AutoSize = true, Font = Theme.Bold(9.5f), ForeColor = tone, BackColor = soft, Padding = new Padding(10, 4, 12, 4), Margin = new Padding(0, 0, 0, 12) };
    }

    private Control Summary()
    {
        var box = new Label
        {
            AutoSize = true,
            Font = Theme.Body(10),
            ForeColor = Theme.Ink,
            BackColor = Theme.Inside,
            Padding = new Padding(16, 12, 16, 12),
            Margin = new Padding(0),
            Text = "หน้าจัดการ   http://localhost:" + panel.Value + "/synaptaconnect\n"
                + "Agent API     TCP " + api.Value + "  ·  ค้นหาอุปกรณ์ UDP " + discovery.Value,
        };
        return box;
    }

    // ---------- การทำงาน ----------

    private async void StartInstall(object sender, EventArgs e)
    {
        if (api.Value == panel.Value) { status.Text = "พอร์ต Agent API และหน้าจัดการต้องไม่ซ้ำกัน"; return; }
        // เจ้าของ = บัญชีที่เปิดตัวติดตั้ง (ก่อนยกสิทธิ์) — บัญชีนี้จะเปิด panel ได้ แม้ UAC จะใช้รหัสผู้ดูแลคนอื่น
        string owner = WindowsIdentity.GetCurrent().Name;
        string arguments = "--install " + api.Value + " " + discovery.Value + " " + panel.Value + " \"" + owner + "\"";
        ShowProgress("กำลังติดตั้ง", "สร้าง service ตั้งสิทธิ์โฟลเดอร์ และเปิดไฟร์วอลล์เฉพาะเครือข่ายส่วนตัว อาจใช้เวลาราวหนึ่งนาที");
        int code = await RunElevated(arguments);
        if (code == 0) { installed = embedded; ShowDone(false); OpenPanel(); return; }
        ShowSetup();
        status.Text = code == 1223 ? "ยกเลิกการขอสิทธิ์ผู้ดูแลแล้ว" : "ติดตั้งไม่สำเร็จ (รหัส " + code + ") ดูรายละเอียดในหน้าต่างแจ้งเตือน";
    }

    private async void StartUninstall(object sender, EventArgs e)
    {
        string arguments = purge.Checked ? "--uninstall --purge" : "--uninstall";
        ShowProgress("กำลังถอนการติดตั้ง", "หยุด service ลบกฎไฟร์วอลล์ และนำโปรแกรมออกจากเครื่อง");
        int code = await RunElevated(arguments);
        // exit 0 = agent ถอนครบแล้ว ส่วนไฟล์อาจหายตามหลังไม่กี่วินาที จึงรอดูก่อนตัดสิน
        for (int i = 0; code == 0 && i < 20 && File.Exists(Setup.InstalledExe); i++) await Task.Delay(500);
        if (code == 0) { installed = null; ShowSetup(); return; }
        ShowInstalled();
        status.Text = code == 1223 ? "ยกเลิกการขอสิทธิ์ผู้ดูแลแล้ว" : "ถอนการติดตั้งไม่สำเร็จ (รหัส " + code + ")";
    }

    private async void StartUpgrade(object sender, EventArgs e)
    {
        ShowProgress("กำลังอัปเดต", "หยุด service ชั่วคราว เปลี่ยนโปรแกรมเป็นรุ่นใหม่ แล้วเริ่มกลับ ใช้เวลาไม่ถึงหนึ่งนาที");
        int code = await RunElevated("--upgrade");
        if (code == 0) { installed = Setup.InstalledVersion(); ShowDone(true); OpenPanel(); return; }
        installed = Setup.InstalledVersion();
        ShowInstalled();
        status.Text = code == 1223 ? "ยกเลิกการขอสิทธิ์ผู้ดูแลแล้ว" : "อัปเดตไม่สำเร็จ (รหัส " + code + ") ยังใช้รุ่นเดิมอยู่";
    }

    /// เรียกตัวเองซ้ำด้วย runas แล้วรอผล — 1223 = ผู้ใช้กดยกเลิกหน้าต่าง UAC
    private static Task<int> RunElevated(string arguments)
    {
        return Task.Run(() =>
        {
            try
            {
                var start = new ProcessStartInfo(Application.ExecutablePath, arguments) { UseShellExecute = true, Verb = "runas" };
                using (var child = Process.Start(start))
                {
                    child.WaitForExit();
                    return child.ExitCode;
                }
            }
            catch (System.ComponentModel.Win32Exception error) { return error.NativeErrorCode == 1223 ? 1223 : 1; }
        });
    }

    private void OpenPanel()
    {
        try
        {
            using (Process.Start(new ProcessStartInfo(Setup.InstalledExe, "panel --gui")
                { UseShellExecute = false, CreateNoWindow = true })) { }
            status.ForeColor = Theme.Good;
            status.Text = "กำลังเปิด Synapta Connect Desktop...";
        }
        catch (Exception error)
        {
            status.ForeColor = Theme.Danger;
            status.Text = error.Message;
        }
    }

    private static NumericUpDown Port(int value)
    {
        return new NumericUpDown
        {
            Minimum = 1,
            Maximum = 65535,
            Value = value,
            Width = 104,
            TextAlign = HorizontalAlignment.Center,
            Font = Theme.Bold(12),
            BorderStyle = BorderStyle.FixedSingle,
            Margin = new Padding(0),
        };
    }
}
