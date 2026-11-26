using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;

namespace VictoryLauncher
{
    sealed class GameVersion
    {
        public string Title;
        public string Subtitle;
        public string Badge;
        public Color BadgeColor;
        public string Kind; // "ps1" or "bat"
        public string Target;
        public string Args;
        public bool SupportsOptions;
        public bool Available;
    }

    sealed class KeyBindingItem
    {
        public string Id;
        public string Label;
        public string DefaultKey;
        public string Key;
        public Color Accent;
    }

    sealed class KeymapForm : Form
    {
        private static readonly Color BgWindow = Color.FromArgb(14, 16, 22);
        private static readonly Color BgCard = Color.FromArgb(22, 26, 36);
        private static readonly Color BgCardHover = Color.FromArgb(30, 36, 50);
        private static readonly Color BorderSubtle = Color.FromArgb(38, 46, 64);
        private static readonly Color AccentCyan = Color.FromArgb(34, 211, 238);
        private static readonly Color AccentPink = Color.FromArgb(244, 63, 148);
        private static readonly Color AccentGreen = Color.FromArgb(52, 211, 153);
        private static readonly Color AccentAmber = Color.FromArgb(251, 191, 36);
        private static readonly Color TextPrimary = Color.FromArgb(241, 245, 249);
        private static readonly Color TextMuted = Color.FromArgb(148, 163, 184);

        private readonly List<KeyBindingItem> _items;
        private readonly float _scale;
        private Rectangle[] _rowRects;
        private Rectangle _resetRect;
        private Rectangle _cancelRect;
        private Rectangle _saveRect;
        private int _hoverRow = -1;
        private int _hoverBtn = -1; // 0=Save, 1=Reset, 2=Cancel
        private int _listeningIndex = -1;
        private string _hint = "Clique em um botão do Xbox e pressione a tecla desejada (ESC cancela • DEL desativa).";

        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int attrValue, int attrSize);

        private int S(int v) { return (int)Math.Round(v * _scale); }

        public KeymapForm(List<KeyBindingItem> items, float scale)
        {
            _items = items;
            _scale = scale;
            Text = "Mapear Controles — Xbox → Teclado";
            StartPosition = FormStartPosition.CenterParent;
            FormBorderStyle = FormBorderStyle.FixedDialog;
            MaximizeBox = false;
            MinimizeBox = false;
            BackColor = BgWindow;
            ForeColor = TextPrimary;
            DoubleBuffered = true;
            KeyPreview = true;
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.OptimizedDoubleBuffer, true);

            int rowsPerCol = (_items.Count + 1) / 2;
            ClientSize = new Size(S(640), S(74) + rowsPerCol * S(32) + S(84));
            ComputeLayout();

            MouseMove += OnMouseMoveCanvas;
            MouseDown += OnMouseDownCanvas;
            KeyDown += OnKeyDownCanvas;
            Shown += (s, e) =>
            {
                try
                {
                    int dark = 1;
                    DwmSetWindowAttribute(Handle, 20, ref dark, sizeof(int));
                    int corner = 2;
                    DwmSetWindowAttribute(Handle, 33, ref corner, sizeof(int));
                }
                catch { }
            };
        }

        private void ComputeLayout()
        {
            _rowRects = new Rectangle[_items.Count];
            int pad = S(20);
            int gap = S(14);
            int colW = (ClientSize.Width - pad * 2 - gap) / 2;
            int rowsPerCol = (_items.Count + 1) / 2;
            int startY = S(62);
            for (int i = 0; i < _items.Count; i++)
            {
                int col = i / rowsPerCol;
                int row = i % rowsPerCol;
                _rowRects[i] = new Rectangle(pad + col * (colW + gap), startY + row * S(32), colW, S(27));
            }
            int btnY = startY + rowsPerCol * S(32) + S(34);
            _resetRect = new Rectangle(pad, btnY, S(162), S(34));
            _cancelRect = new Rectangle(ClientSize.Width - pad - S(268), btnY, S(116), S(34));
            _saveRect = new Rectangle(ClientSize.Width - pad - S(144), btnY, S(144), S(34));
        }

        private static GraphicsPath RoundedRect(Rectangle r, int radius)
        {
            int d = Math.Max(2, radius * 2);
            GraphicsPath path = new GraphicsPath();
            path.AddArc(r.X, r.Y, d, d, 180, 90);
            path.AddArc(r.Right - d, r.Y, d, d, 270, 90);
            path.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
            path.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
            path.CloseFigure();
            return path;
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            Graphics g = e.Graphics;
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

            using (var grad = new LinearGradientBrush(new Rectangle(0, 0, ClientSize.Width, S(3)), AccentPink, AccentCyan, 0f))
            {
                g.FillRectangle(grad, 0, 0, ClientSize.Width, S(3));
            }

            int pad = S(20);
            using (var fTitle = new Font("Segoe UI Semibold", 12f, FontStyle.Bold))
            using (var fSub = new Font("Segoe UI", 8.2f))
            using (var fRow = new Font("Segoe UI", 8.6f))
            using (var fKey = new Font("Segoe UI Semibold", 8.2f, FontStyle.Bold))
            using (var bPri = new SolidBrush(TextPrimary))
            using (var bMut = new SolidBrush(TextMuted))
            using (var sfC = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center })
            {
                g.DrawString("MAPEAR CONTROLES DO XBOX PARA TECLADO", fTitle, bPri, pad, S(14));
                g.DrawString("Ex.: clique em \"X (Azul)\" e pressione K para mapear X = K", fSub, bMut, pad, S(38));

                for (int i = 0; i < _items.Count; i++)
                {
                    var item = _items[i];
                    Rectangle r = _rowRects[i];
                    bool listening = (i == _listeningIndex);
                    bool hover = (i == _hoverRow);

                    Color bg = listening ? Color.FromArgb(42, AccentPink) : (hover ? BgCardHover : BgCard);
                    Color border = listening ? AccentPink : (hover ? AccentCyan : BorderSubtle);
                    using (var p = RoundedRect(r, S(6)))
                    using (var bBg = new SolidBrush(bg))
                    using (var pen = new Pen(border, listening ? 1.4f : 1f))
                    {
                        g.FillPath(bBg, p);
                        g.DrawPath(pen, p);
                    }

                    // Color dot for Xbox button
                    using (var bDot = new SolidBrush(item.Accent))
                    {
                        g.FillEllipse(bDot, r.X + S(9), r.Y + S(9), S(8), S(8));
                    }
                    g.DrawString(item.Label, fRow, bPri, r.X + S(24), r.Y + S(5));

                    // Key badge on right
                    string keyText = listening ? "APERTE TECLA..." : (string.IsNullOrEmpty(item.Key) || item.Key == "NONE" ? "DESATIVADO" : item.Key);
                    bool modified = !string.Equals(item.Key, item.DefaultKey, StringComparison.OrdinalIgnoreCase);
                    Color kColor = listening ? AccentPink : (modified ? AccentCyan : AccentGreen);
                    Rectangle kr = new Rectangle(r.Right - S(104), r.Y + S(3), S(98), r.Height - S(6));
                    using (var kp = RoundedRect(kr, S(5)))
                    using (var bKBg = new SolidBrush(Color.FromArgb(30, kColor)))
                    using (var pK = new Pen(Color.FromArgb(110, kColor), 1f))
                    using (var bKTxt = new SolidBrush(kColor))
                    {
                        g.FillPath(bKBg, kp);
                        g.DrawPath(pK, kp);
                        g.DrawString(keyText, fKey, bKTxt, kr, sfC);
                    }
                }

                // Hint bar
                int rowsPerCol = (_items.Count + 1) / 2;
                int hintY = S(62) + rowsPerCol * S(32) + S(8);
                using (var bHint = new SolidBrush(_listeningIndex >= 0 ? AccentPink : AccentAmber))
                {
                    g.DrawString(_hint, fSub, bHint, pad, hintY);
                }

                DrawBtn(g, _resetRect, "Restaurar Padrão", false, _hoverBtn == 1, sfC);
                DrawBtn(g, _cancelRect, "Cancelar", false, _hoverBtn == 2, sfC);
                DrawBtn(g, _saveRect, "Salvar Controles", true, _hoverBtn == 0, sfC);
            }
        }

        private void DrawBtn(Graphics g, Rectangle r, string text, bool primary, bool hover, StringFormat sf)
        {
            using (var path = RoundedRect(r, S(8)))
            {
                if (primary)
                {
                    using (var grad = new LinearGradientBrush(r, hover ? Color.FromArgb(255, 86, 164) : AccentPink, hover ? Color.FromArgb(56, 228, 250) : AccentCyan, 0f))
                    {
                        g.FillPath(grad, path);
                    }
                }
                else
                {
                    using (var bBg = new SolidBrush(hover ? BgCardHover : BgCard))
                    using (var pen = new Pen(hover ? AccentCyan : BorderSubtle, 1f))
                    {
                        g.FillPath(bBg, path);
                        g.DrawPath(pen, path);
                    }
                }
            }
            using (var f = new Font("Segoe UI Semibold", 8.8f, FontStyle.Bold))
            using (var b = new SolidBrush(primary ? Color.FromArgb(10, 12, 18) : TextPrimary))
            {
                g.DrawString(text, f, b, r, sf);
            }
        }

        private void OnMouseMoveCanvas(object sender, MouseEventArgs e)
        {
            int oldRow = _hoverRow;
            int oldBtn = _hoverBtn;
            _hoverRow = -1;
            _hoverBtn = -1;
            for (int i = 0; i < _rowRects.Length; i++)
            {
                if (_rowRects[i].Contains(e.Location)) { _hoverRow = i; break; }
            }
            if (_saveRect.Contains(e.Location)) _hoverBtn = 0;
            else if (_resetRect.Contains(e.Location)) _hoverBtn = 1;
            else if (_cancelRect.Contains(e.Location)) _hoverBtn = 2;
            Cursor = (_hoverRow >= 0 || _hoverBtn >= 0) ? Cursors.Hand : Cursors.Default;
            if (oldRow != _hoverRow || oldBtn != _hoverBtn) Invalidate();
        }

        private void OnMouseDownCanvas(object sender, MouseEventArgs e)
        {
            if (e.Button != MouseButtons.Left) return;
            for (int i = 0; i < _rowRects.Length; i++)
            {
                if (_rowRects[i].Contains(e.Location))
                {
                    _listeningIndex = (_listeningIndex == i) ? -1 : i;
                    _hint = _listeningIndex >= 0
                        ? "Aguardando tecla para \"" + _items[i].Label + "\"... (ESC cancela • DEL desativa)"
                        : "Clique em um botão do Xbox e pressione a tecla desejada.";
                    Invalidate();
                    return;
                }
            }
            if (_resetRect.Contains(e.Location))
            {
                _listeningIndex = -1;
                foreach (var it in _items) it.Key = it.DefaultKey;
                _hint = "Todos os botões restaurados para o padrão do Victory.";
                Invalidate();
            }
            else if (_cancelRect.Contains(e.Location))
            {
                DialogResult = DialogResult.Cancel;
                Close();
            }
            else if (_saveRect.Contains(e.Location))
            {
                DialogResult = DialogResult.OK;
                Close();
            }
        }

        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            if (_listeningIndex >= 0)
            {
                Keys code = keyData & Keys.KeyCode;
                HandleCapturedKey(code);
                return true;
            }
            return base.ProcessCmdKey(ref msg, keyData);
        }

        private void OnKeyDownCanvas(object sender, KeyEventArgs e)
        {
            if (_listeningIndex >= 0)
            {
                HandleCapturedKey(e.KeyCode);
                e.Handled = true;
                e.SuppressKeyPress = true;
                return;
            }
            if (e.KeyCode == Keys.Escape)
            {
                DialogResult = DialogResult.Cancel;
                Close();
            }
        }

        private void HandleCapturedKey(Keys code)
        {
            if (_listeningIndex < 0) return;
            if (code == Keys.Escape)
            {
                _listeningIndex = -1;
                _hint = "Captura cancelada.";
                Invalidate();
                return;
            }
            string mapped = KeyToSpec(code);
            if (code == Keys.Delete)
            {
                mapped = "NONE";
            }
            if (string.IsNullOrEmpty(mapped))
            {
                _hint = "Tecla não suportada (" + code + "). Tente outra tecla.";
                Invalidate();
                return;
            }
            var target = _items[_listeningIndex];
            string cleared = null;
            if (mapped != "NONE")
            {
                foreach (var other in _items)
                {
                    if (other != target && string.Equals(other.Key, mapped, StringComparison.OrdinalIgnoreCase))
                    {
                        other.Key = "NONE";
                        cleared = other.Label;
                    }
                }
            }
            target.Key = mapped;
            _listeningIndex = -1;
            _hint = mapped == "NONE"
                ? "\"" + target.Label + "\" desativado."
                : ("Mapeado: " + target.Label + " = " + mapped + (cleared != null ? " (removido de \"" + cleared + "\")" : ""));
            Invalidate();
        }

        private static string KeyToSpec(Keys code)
        {
            if (code >= Keys.A && code <= Keys.Z) return code.ToString();
            if (code >= Keys.D0 && code <= Keys.D9) return ((char)('0' + (code - Keys.D0))).ToString();
            if (code >= Keys.NumPad0 && code <= Keys.NumPad9) return "NUM" + (code - Keys.NumPad0);
            if (code >= Keys.F1 && code <= Keys.F12) return "F" + (1 + (code - Keys.F1));
            switch (code)
            {
                case Keys.Space: return "SPACE";
                case Keys.Return: return "ENTER";
                case Keys.Tab: return "TAB";
                case Keys.Back: return "BACKSPACE";
                case Keys.ShiftKey:
                case Keys.LShiftKey:
                case Keys.RShiftKey: return "SHIFT";
                case Keys.ControlKey:
                case Keys.LControlKey:
                case Keys.RControlKey: return "CTRL";
                case Keys.Menu:
                case Keys.LMenu:
                case Keys.RMenu: return "ALT";
                case Keys.CapsLock: return "CAPSLOCK";
                case Keys.Up: return "UP";
                case Keys.Down: return "DOWN";
                case Keys.Left: return "LEFT";
                case Keys.Right: return "RIGHT";
                case Keys.Insert: return "INSERT";
                case Keys.Home: return "HOME";
                case Keys.End: return "END";
                case Keys.PageUp: return "PGUP";
                case Keys.PageDown: return "PGDN";
                case Keys.Oemcomma: return "COMMA";
                case Keys.OemPeriod: return "PERIOD";
                case Keys.OemSemicolon: return "SEMICOLON";
                case Keys.OemQuestion: return "SLASH";
                case Keys.OemMinus: return "MINUS";
                case Keys.Oemplus: return "EQUALS";
                case Keys.OemOpenBrackets: return "LBRACKET";
                case Keys.OemCloseBrackets: return "RBRACKET";
                case Keys.OemPipe: return "BACKSLASH";
                case Keys.OemQuotes: return "QUOTE";
                case Keys.Oemtilde: return "TILDE";
                default:
                    int vk = (int)code;
                    if (vk >= 8 && vk <= 254) return "0x" + vk.ToString("X2");
                    return null;
            }
        }
    }

    sealed class LauncherForm : Form
    {
        private const string GameDir = @"C:\Users\gabri\Downloads\GTA6.Xbox.v1.0.2.1-CYBERLEEK";
        private const string RepoDir = @"C:\Users\gabri\Downloads\victory-klbf-lab39";
        private const string DataDir = @"C:\Users\gabri\Downloads\victory-data";
        private const string ConfigPath = @"C:\Users\gabri\Downloads\victory-data\launcher-config.ini";
        private const string Ps1Path = @"C:\Users\gabri\Downloads\victory-klbf-lab39\run-klbf-lab39.ps1";
        private const string PrecompileBat = @"C:\Users\gabri\Downloads\GTA6.Xbox.v1.0.2.1-CYBERLEEK\PreCompilar-Shaders-KLBF-Lab39-AMD.bat";

        private static readonly Color BgWindow = Color.FromArgb(14, 16, 22);
        private static readonly Color BgCard = Color.FromArgb(22, 26, 36);
        private static readonly Color BgCardHover = Color.FromArgb(28, 34, 47);
        private static readonly Color BgCardSelected = Color.FromArgb(30, 38, 54);
        private static readonly Color BgPanel = Color.FromArgb(18, 22, 31);
        private static readonly Color BorderSubtle = Color.FromArgb(38, 46, 64);
        private static readonly Color AccentCyan = Color.FromArgb(34, 211, 238);
        private static readonly Color AccentPink = Color.FromArgb(244, 63, 148);
        private static readonly Color AccentGreen = Color.FromArgb(52, 211, 153);
        private static readonly Color AccentAmber = Color.FromArgb(251, 191, 36);
        private static readonly Color AccentBlue = Color.FromArgb(96, 165, 250);
        private static readonly Color AccentRed = Color.FromArgb(248, 113, 113);
        private static readonly Color AccentSlate = Color.FromArgb(148, 163, 184);
        private static readonly Color TextPrimary = Color.FromArgb(241, 245, 249);
        private static readonly Color TextMuted = Color.FromArgb(148, 163, 184);

        private readonly List<GameVersion> _versions = new List<GameVersion>();
        private readonly List<KeyBindingItem> _keymap = new List<KeyBindingItem>();
        private int _selectedIndex = 0;
        private int _hoverIndex = -1;
        // 0=Play, 1=Precompile, 2=Logs, 3=HudChk, 4=CloseChk, 5=MouseCamChk, 6=InvertYChk,
        // 7=SensTrack, 8=BtnRmbShoot, 9=BtnLmbShoot, 10=BtnOff, 11=UnifiedMemChk, 12=KeymapBtn
        private int _hoverCtrl = -1;
        private bool _draggingSens = false;

        private bool _enableHud = true;
        private bool _closeOnLaunch = true;
        private bool _unifiedMemory = true;
        private bool _liteGraphics = false;
        private bool _mouseCamera = false;
        private bool _mouseInvertY = false;
        private float _mouseSens = 1.0f; // 0.2 .. 4.0
        private string _mouseButtons = "rmb_shoot"; // "rmb_shoot", "lmb_shoot", "off"
        private bool _cachePrecompiled = false;
        private string _statusText = "";
        private float _scale = 1f;

        private int S(int v) { return (int)Math.Round(v * _scale); }

        private Rectangle[] _cardRects = new Rectangle[0];
        private Rectangle _mousePanelRect;
        private Rectangle _mouseCamChkRect;
        private Rectangle _invertYChkRect;
        private Rectangle _sensTrackRect;
        private Rectangle _btnRmbRect;
        private Rectangle _btnLmbRect;
        private Rectangle _btnOffRect;
        private Rectangle _keymapBtnRect;
        private Rectangle _unifiedChkRect;
        private Rectangle _liteChkRect;
        private Rectangle _hudChkRect;
        private Rectangle _closeChkRect;
        private Rectangle _precompileRect;
        private Rectangle _logsRect;
        private Rectangle _playRect;

        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int attrValue, int attrSize);

        [DllImport("kernel32.dll")]
        private static extern bool SetProcessWorkingSetSize(IntPtr proc, IntPtr min, IntPtr max);

        [DllImport("psapi.dll")]
        private static extern bool EmptyWorkingSet(IntPtr hProcess);

        [DllImport("user32.dll")]
        private static extern bool SetProcessDPIAware();

        public LauncherForm()
        {
            Text = "GTA VI — VIctory Launcher";
            StartPosition = FormStartPosition.CenterScreen;
            FormBorderStyle = FormBorderStyle.FixedSingle;
            MaximizeBox = false;
            BackColor = BgWindow;
            ForeColor = TextPrimary;
            DoubleBuffered = true;
            KeyPreview = true;
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.OptimizedDoubleBuffer, true);

            using (var g = CreateGraphics()) { _scale = g.DpiX / 96f; }
            _cachePrecompiled = File.Exists(Path.Combine(DataDir, "cache-klbf-lab39", "precompiled.ok"));
            InitDefaultKeymap();
            LoadVersions();
            LoadConfig();

            int listH = _versions.Count * S(54) + (_versions.Count - 1) * S(6);
            ClientSize = new Size(S(680), S(104) + listH + S(178) + S(140));
            ComputeLayout();

            MouseMove += OnMouseMoveCanvas;
            MouseLeave += (s, e) => { if (!_draggingSens) { _hoverIndex = -1; _hoverCtrl = -1; Cursor = Cursors.Default; Invalidate(); } };
            MouseDown += OnMouseDownCanvas;
            MouseUp += (s, e) => { if (_draggingSens) { _draggingSens = false; SaveConfig(); Invalidate(); } };
            MouseDoubleClick += OnMouseDoubleClickCanvas;
            KeyDown += OnKeyDownCanvas;
            Shown += (s, e) => { EnableDarkTitleBar(); TrimMemory(); };
        }

        private void InitDefaultKeymap()
        {
            _keymap.Clear();
            Action<string, string, string, Color> add = (id, label, def, col) =>
            {
                _keymap.Add(new KeyBindingItem { Id = id, Label = label, DefaultKey = def, Key = def, Accent = col });
            };
            add("A", "Botão A (Verde)", "ENTER", AccentGreen);
            add("B", "Botão B (Vermelho)", "BACKSPACE", AccentRed);
            add("X", "Botão X (Azul)", "X", AccentBlue);
            add("Y", "Botão Y (Amarelo)", "Y", AccentAmber);
            add("LB", "LB / L1 (Ombro Esq)", "Q", AccentCyan);
            add("RB", "RB / R1 (Ombro Dir)", "E", AccentCyan);
            add("LT", "LT / L2 (Gatilho Esq)", "Z", AccentPink);
            add("RT", "RT / R2 (Gatilho Dir)", "C", AccentPink);
            add("MENU", "Start / Menu", "ESC", AccentSlate);
            add("VIEW", "Back / View", "TAB", AccentSlate);
            add("LS", "L3 (Analógico Esq)", "F", AccentSlate);
            add("RS", "R3 (Analógico Dir)", "H", AccentSlate);

            add("UP", "D-Pad Cima", "UP", AccentCyan);
            add("DOWN", "D-Pad Baixo", "DOWN", AccentCyan);
            add("LEFT", "D-Pad Esquerda", "LEFT", AccentCyan);
            add("RIGHT", "D-Pad Direita", "RIGHT", AccentCyan);
            add("LY+", "Mover Frente (LS ↑)", "W", AccentGreen);
            add("LY-", "Mover Trás (LS ↓)", "S", AccentGreen);
            add("LX-", "Mover Esquerda (LS ←)", "A", AccentGreen);
            add("LX+", "Mover Direita (LS →)", "D", AccentGreen);
            add("RY+", "Câmera Cima (RS ↑)", "I", AccentAmber);
            add("RY-", "Câmera Baixo (RS ↓)", "K", AccentAmber);
            add("RX-", "Câmera Esquerda (RS ←)", "J", AccentAmber);
            add("RX+", "Câmera Direita (RS →)", "L", AccentAmber);
        }

        private string SerializeCustomKeymap()
        {
            var diffs = new List<string>();
            foreach (var it in _keymap)
            {
                if (!string.Equals(it.Key, it.DefaultKey, StringComparison.OrdinalIgnoreCase))
                {
                    diffs.Add(it.Id + "=" + (string.IsNullOrEmpty(it.Key) ? "NONE" : it.Key));
                }
            }
            return string.Join(",", diffs.ToArray());
        }

        private void ApplyKeymapString(string spec)
        {
            if (string.IsNullOrWhiteSpace(spec)) return;
            foreach (string pair in spec.Split(new[] { ',', ';' }, StringSplitOptions.RemoveEmptyEntries))
            {
                int eq = pair.IndexOf('=');
                if (eq <= 0) continue;
                string id = pair.Substring(0, eq).Trim();
                string val = pair.Substring(eq + 1).Trim().ToUpperInvariant();
                foreach (var it in _keymap)
                {
                    if (string.Equals(it.Id, id, StringComparison.OrdinalIgnoreCase))
                    {
                        it.Key = val;
                        break;
                    }
                }
            }
        }

        private void EnableDarkTitleBar()
        {
            try
            {
                int dark = 1;
                DwmSetWindowAttribute(Handle, 20, ref dark, sizeof(int));
                int corner = 2;
                DwmSetWindowAttribute(Handle, 33, ref corner, sizeof(int));
            }
            catch { }
        }

        private static void TrimMemory()
        {
            try
            {
                GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, true);
                GC.WaitForPendingFinalizers();
                IntPtr h = Process.GetCurrentProcess().Handle;
                EmptyWorkingSet(h);
                SetProcessWorkingSetSize(h, (IntPtr)(-1), (IntPtr)(-1));
            }
            catch { }
        }

        private void LoadConfig()
        {
            try
            {
                if (!File.Exists(ConfigPath)) return;
                foreach (string raw in File.ReadAllLines(ConfigPath))
                {
                    int eq = raw.IndexOf('=');
                    if (eq <= 0) continue;
                    string k = raw.Substring(0, eq).Trim();
                    string v = raw.Substring(eq + 1).Trim();
                    if (k == "selected")
                    {
                        int idx;
                        if (int.TryParse(v, out idx) && idx >= 0 && idx < _versions.Count) _selectedIndex = idx;
                    }
                    else if (k == "hud") _enableHud = (v == "1");
                    else if (k == "close_on_launch") _closeOnLaunch = (v == "1");
                    else if (k == "unified_memory") _unifiedMemory = (v != "0");
                    else if (k == "lite_graphics") _liteGraphics = (v == "1");
                    else if (k == "mouse_camera") _mouseCamera = (v == "1");
                    else if (k == "mouse_invert_y") _mouseInvertY = (v == "1");
                    else if (k == "mouse_sens")
                    {
                        float s;
                        if (float.TryParse(v, NumberStyles.Float, CultureInfo.InvariantCulture, out s))
                            _mouseSens = Math.Max(0.2f, Math.Min(4.0f, s));
                    }
                    else if (k == "mouse_buttons" && (v == "rmb_shoot" || v == "lmb_shoot" || v == "off"))
                    {
                        _mouseButtons = v;
                    }
                    else if (k == "keymap")
                    {
                        ApplyKeymapString(v);
                    }
                }
            }
            catch { }
        }

        private void SaveConfig()
        {
            try
            {
                Directory.CreateDirectory(DataDir);
                string[] lines = new string[]
                {
                    "selected=" + _selectedIndex,
                    "hud=" + (_enableHud ? "1" : "0"),
                    "close_on_launch=" + (_closeOnLaunch ? "1" : "0"),
                    "unified_memory=" + (_unifiedMemory ? "1" : "0"),
                    "lite_graphics=" + (_liteGraphics ? "1" : "0"),
                    "mouse_camera=" + (_mouseCamera ? "1" : "0"),
                    "mouse_invert_y=" + (_mouseInvertY ? "1" : "0"),
                    "mouse_sens=" + _mouseSens.ToString("0.00", CultureInfo.InvariantCulture),
                    "mouse_buttons=" + _mouseButtons,
                    "keymap=" + SerializeCustomKeymap()
                };
                File.WriteAllLines(ConfigPath, lines);
            }
            catch { }
        }

        private void LoadVersions()
        {
            bool ps1Ok = File.Exists(Ps1Path) && File.Exists(Path.Combine(RepoDir, "target", "release", "victory.exe"));

            _versions.Add(new GameVersion
            {
                Title = "Unificado FPS MAX — FSR 2.4 (720p)",
                Subtitle = "UMA + Tiled Physical Pool + KLBF 39 + LATE_WAIT=3 + Batch Paralelo",
                Badge = "RECOMENDADO",
                BadgeColor = AccentCyan,
                Kind = "ps1",
                Target = Ps1Path,
                Args = "-Res720p",
                SupportsOptions = true,
                Available = ps1Ok
            });

            _versions.Add(new GameVersion
            {
                Title = "Unificado FPS MAX — FSR 2.4 (1080p)",
                Subtitle = "Full HD nativo com AMD FSR 2.4 • Água KLBF 39 + 524k slots AMD",
                Badge = "1080p FSR",
                BadgeColor = AccentGreen,
                Kind = "ps1",
                Target = Ps1Path,
                Args = "",
                SupportsOptions = true,
                Available = ps1Ok
            });

            _versions.Add(new GameVersion
            {
                Title = "Unificado FPS MAX — Simple (720p)",
                Subtitle = "Upscaler bilinear ultraleve sem FSR • Menor consumo de VRAM e RAM",
                Badge = "MAIS LEVE",
                BadgeColor = AccentGreen,
                Kind = "ps1",
                Target = Ps1Path,
                Args = "-SimpleUpscale -Res720p",
                SupportsOptions = true,
                Available = ps1Ok
            });

            _versions.Add(new GameVersion
            {
                Title = "Unificado FPS MAX — Simple (1080p)",
                Subtitle = "Full HD com upscaler bilinear simples • Estável e sem overhead de FSR",
                Badge = "1080p",
                BadgeColor = AccentSlate,
                Kind = "ps1",
                Target = Ps1Path,
                Args = "-SimpleUpscale",
                SupportsOptions = true,
                Available = ps1Ok
            });

            _versions.Add(new GameVersion
            {
                Title = "Unificado — FSR 2.4 + FrameGen (720p)",
                Subtitle = "Geração de quadros AMD (+500 MB VRAM) • Experimental no mundo aberto",
                Badge = "FRAMEGEN",
                BadgeColor = AccentPink,
                Kind = "ps1",
                Target = Ps1Path,
                Args = "-FrameGen -Res720p",
                SupportsOptions = true,
                Available = ps1Ok
            });

            _versions.Add(new GameVersion
            {
                Title = "Unificado — Modo Seguro (720p)",
                Subtitle = "Sem flags agressivas (LATE_WAIT=1, CopyPlain=0) • Máxima estabilidade",
                Badge = "SEGURO",
                BadgeColor = AccentAmber,
                Kind = "ps1",
                Target = Ps1Path,
                Args = "-Safe -Res720p",
                SupportsOptions = true,
                Available = ps1Ok
            });

            string backupBat = Path.Combine(GameDir, "Jogar-GTA6-RX6600-Otimizado-HUD.bat");
            if (File.Exists(backupBat))
            {
                _versions.Add(new GameVersion
                {
                    Title = "RX 6600 Original (Backup)",
                    Subtitle = "Build anterior intacta • Jogar-GTA6-RX6600-Otimizado-HUD.bat",
                    Badge = "BACKUP",
                    BadgeColor = AccentSlate,
                    Kind = "bat",
                    Target = backupBat,
                    Args = "",
                    SupportsOptions = false,
                    Available = true
                });
            }
        }

        private void ComputeLayout()
        {
            _cardRects = new Rectangle[_versions.Count];
            int pad = S(22);
            int y = S(98);
            int cardW = ClientSize.Width - pad * 2;
            for (int i = 0; i < _versions.Count; i++)
            {
                _cardRects[i] = new Rectangle(pad, y, cardW, S(54));
                y += S(60);
            }

            // Mouse & Keyboard Controls Panel
            int panelY = y + S(10);
            _mousePanelRect = new Rectangle(pad, panelY, cardW, S(156));
            _mouseCamChkRect = new Rectangle(pad + S(14), panelY + S(28), S(390), S(22));
            _invertYChkRect = new Rectangle(pad + cardW - S(155), panelY + S(28), S(140), S(22));
            _sensTrackRect = new Rectangle(pad + S(128), panelY + S(58), cardW - S(196), S(18));

            int btnRowY = panelY + S(84);
            int segW = (cardW - S(28) - S(128) - S(12)) / 3;
            int segX = pad + S(128);
            _btnRmbRect = new Rectangle(segX, btnRowY, segW, S(24));
            _btnLmbRect = new Rectangle(segX + segW + S(6), btnRowY, segW, S(24));
            _btnOffRect = new Rectangle(segX + (segW + S(6)) * 2, btnRowY, segW, S(24));

            int kmRowY = panelY + S(118);
            _keymapBtnRect = new Rectangle(pad + S(14), kmRowY, S(212), S(26));

            // Footer checkboxes (Row 1: Unified Memory; Row 2: Lite Graphics; Row 3: HUD + Close)
            int footerY = _mousePanelRect.Bottom + S(12);
            _unifiedChkRect = new Rectangle(pad, footerY, cardW, S(24));
            int footerY15 = footerY + S(26);
            _liteChkRect = new Rectangle(pad, footerY15, cardW, S(24));
            int footerY2 = footerY15 + S(26);
            _hudChkRect = new Rectangle(pad, footerY2, S(185), S(24));
            _closeChkRect = new Rectangle(pad + S(196), footerY2, S(250), S(24));

            int btnY = footerY2 + S(32);
            _precompileRect = new Rectangle(pad, btnY, S(186), S(38));
            _logsRect = new Rectangle(pad + S(196), btnY, S(120), S(38));
            _playRect = new Rectangle(ClientSize.Width - pad - S(200), btnY, S(200), S(38));
        }

        private static GraphicsPath RoundedRect(Rectangle r, int radius)
        {
            int d = Math.Max(2, radius * 2);
            GraphicsPath path = new GraphicsPath();
            path.AddArc(r.X, r.Y, d, d, 180, 90);
            path.AddArc(r.Right - d, r.Y, d, d, 270, 90);
            path.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
            path.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
            path.CloseFigure();
            return path;
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            Graphics g = e.Graphics;
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

            using (var grad = new LinearGradientBrush(new Rectangle(0, 0, ClientSize.Width, S(3)), AccentPink, AccentCyan, 0f))
            {
                g.FillRectangle(grad, 0, 0, ClientSize.Width, S(3));
            }

            int pad = S(22);

            // Header
            using (var fTitle = new Font("Segoe UI Semibold", 16.5f, FontStyle.Bold))
            using (var fSub = new Font("Segoe UI", 8.8f))
            using (var fPill = new Font("Segoe UI Semibold", 7.6f, FontStyle.Bold))
            using (var bTitle = new SolidBrush(TextPrimary))
            using (var bMuted = new SolidBrush(TextMuted))
            {
                g.DrawString("GTA VI", fTitle, bTitle, pad, S(16));
                SizeF gtaSz = g.MeasureString("GTA VI", fTitle);
                using (var bAccent = new SolidBrush(AccentCyan))
                {
                    g.DrawString("VICTORY", fTitle, bAccent, pad + gtaSz.Width + S(2), S(16));
                }

                string memBadge = _unifiedMemory ? "UMA + Tiled Pool" : "Modo Clássico AMD";
                g.DrawString("RX 6600 8 GB  •  Ryzen 5 4500  •  " + memBadge, fSub, bMuted, pad, S(47));

                string cacheLabel = _cachePrecompiled ? "CACHE SHADERS OK" : "CACHE FRIO (PRÉ-COMPILE)";
                Color cacheColor = _cachePrecompiled ? AccentGreen : AccentAmber;
                SizeF sz = g.MeasureString(cacheLabel, fPill);
                Rectangle pill = new Rectangle(ClientSize.Width - pad - (int)sz.Width - S(20), S(22), (int)sz.Width + S(20), S(22));
                using (var pPath = RoundedRect(pill, S(11)))
                using (var bBg = new SolidBrush(Color.FromArgb(26, cacheColor)))
                using (var pPen = new Pen(Color.FromArgb(110, cacheColor), 1f))
                using (var bTxt = new SolidBrush(cacheColor))
                {
                    g.FillPath(bBg, pPath);
                    g.DrawPath(pPen, pPath);
                    g.DrawString(cacheLabel, fPill, bTxt, pill.X + S(10), pill.Y + S(4));
                }

                g.DrawString("VERSÃO DO JOGO", fPill, bMuted, pad, S(78));
            }

            // Version cards
            using (var fCardTitle = new Font("Segoe UI Semibold", 10f, FontStyle.Bold))
            using (var fCardSub = new Font("Segoe UI", 8.2f))
            using (var fBadge = new Font("Segoe UI Semibold", 7.2f, FontStyle.Bold))
            {
                for (int i = 0; i < _versions.Count; i++)
                {
                    GameVersion v = _versions[i];
                    Rectangle r = _cardRects[i];
                    bool sel = (i == _selectedIndex);
                    bool hov = (i == _hoverIndex);

                    Color bg = sel ? BgCardSelected : (hov ? BgCardHover : BgCard);
                    Color border = sel ? AccentCyan : (hov ? Color.FromArgb(65, 78, 104) : BorderSubtle);

                    using (var path = RoundedRect(r, S(9)))
                    using (var bBg = new SolidBrush(bg))
                    using (var pBorder = new Pen(border, sel ? 1.5f : 1f))
                    {
                        g.FillPath(bBg, path);
                        g.DrawPath(pBorder, path);
                    }

                    if (sel)
                    {
                        Rectangle bar = new Rectangle(r.X + 1, r.Y + S(11), S(3), r.Height - S(22));
                        using (var grad = new LinearGradientBrush(bar, AccentPink, AccentCyan, 90f))
                        {
                            g.FillRectangle(grad, bar);
                        }
                    }

                    using (var bT = new SolidBrush(v.Available ? TextPrimary : TextMuted))
                    using (var bS = new SolidBrush(TextMuted))
                    {
                        g.DrawString(v.Title, fCardTitle, bT, r.X + S(14), r.Y + S(8));
                        g.DrawString(v.Subtitle, fCardSub, bS, r.X + S(14), r.Y + S(30));
                    }

                    SizeF bsz = g.MeasureString(v.Badge, fBadge);
                    Rectangle br = new Rectangle(r.Right - (int)bsz.Width - S(24), r.Y + S(9), (int)bsz.Width + S(14), S(18));
                    using (var bp = RoundedRect(br, S(9)))
                    using (var bBg = new SolidBrush(Color.FromArgb(30, v.BadgeColor)))
                    using (var bTxt = new SolidBrush(v.BadgeColor))
                    {
                        g.FillPath(bBg, bp);
                        g.DrawString(v.Badge, fBadge, bTxt, br.X + S(7), br.Y + S(2));
                    }
                }
            }

            // Mouse & Keyboard Controls Panel
            DrawMousePanel(g);

            // Footer checkboxes
            string memLabel = _unifiedMemory
                ? "Memória Unificada (UMA + Tiled Pool) — desmarque p/ Modo Clássico AMD"
                : "Modo Clássico AMD (sem Memória Unificada / como era antes)";
            DrawCheckbox(g, _unifiedChkRect, _unifiedMemory, memLabel, _hoverCtrl == 11);
            DrawCheckbox(g, _liteChkRect, _liteGraphics, "Modo Gráfico Leve (Iluminação Baixa • Shaders Lite • Texturas Leves)", _hoverCtrl == 13);
            DrawCheckbox(g, _hudChkRect, _enableHud, "HUD de desempenho", _hoverCtrl == 3);
            DrawCheckbox(g, _closeChkRect, _closeOnLaunch, "Fechar ao jogar (0 MB RAM)", _hoverCtrl == 4);

            // Buttons
            DrawButton(g, _precompileRect, "Pré-Compilar Shaders", false, _hoverCtrl == 1);
            DrawButton(g, _logsRect, "Abrir Logs", false, _hoverCtrl == 2);
            DrawButton(g, _playRect, "▶   JOGAR AGORA", true, _hoverCtrl == 0);

            if (!string.IsNullOrEmpty(_statusText))
            {
                using (var fSt = new Font("Segoe UI", 8f))
                using (var bSt = new SolidBrush(AccentAmber))
                {
                    g.DrawString(_statusText, fSt, bSt, pad, ClientSize.Height - S(18));
                }
            }
        }

        private void DrawMousePanel(Graphics g)
        {
            using (var path = RoundedRect(_mousePanelRect, S(10)))
            using (var bBg = new SolidBrush(BgPanel))
            using (var pBorder = new Pen(_mouseCamera ? Color.FromArgb(90, AccentCyan) : BorderSubtle, 1f))
            {
                g.FillPath(bBg, path);
                g.DrawPath(pBorder, path);
            }

            using (var fHdr = new Font("Segoe UI Semibold", 7.5f, FontStyle.Bold))
            using (var fLbl = new Font("Segoe UI", 8.5f))
            using (var fVal = new Font("Segoe UI Semibold", 8.5f, FontStyle.Bold))
            using (var bMuted = new SolidBrush(TextMuted))
            using (var bAccent = new SolidBrush(_mouseCamera ? AccentCyan : TextMuted))
            {
                g.DrawString("CONTROLES — MOUSE E MAPEAMENTO XBOX → TECLADO", fHdr, bAccent, _mousePanelRect.X + S(14), _mousePanelRect.Y + S(8));

                // Row 1: Checkboxes
                DrawCheckbox(g, _mouseCamChkRect, _mouseCamera, "Mouse como Câmera (Analógico Direito • F9 trava/solta)", _hoverCtrl == 5);
                DrawCheckbox(g, _invertYChkRect, _mouseInvertY, "Inverter Eixo Y", _hoverCtrl == 6);

                // Row 2: Sensitivity slider
                g.DrawString("Sensibilidade:", fLbl, bMuted, _mousePanelRect.X + S(14), _sensTrackRect.Y + S(1));

                int trackY = _sensTrackRect.Y + _sensTrackRect.Height / 2;
                int trackH = S(4);
                Rectangle bgTrack = new Rectangle(_sensTrackRect.X, trackY - trackH / 2, _sensTrackRect.Width, trackH);
                float norm = (_mouseSens - 0.2f) / (4.0f - 0.2f);
                norm = Math.Max(0f, Math.Min(1f, norm));
                int fillW = Math.Max(S(4), (int)(bgTrack.Width * norm));
                Rectangle fillTrack = new Rectangle(bgTrack.X, bgTrack.Y, fillW, trackH);

                using (var pBg = RoundedRect(bgTrack, S(2)))
                using (var bTr = new SolidBrush(BorderSubtle))
                {
                    g.FillPath(bTr, pBg);
                }
                using (var pFill = RoundedRect(fillTrack, S(2)))
                using (var grad = new LinearGradientBrush(bgTrack, AccentPink, AccentCyan, 0f))
                {
                    g.FillPath(grad, pFill);
                }

                int knobD = S(13);
                int knobX = bgTrack.X + (int)(bgTrack.Width * norm) - knobD / 2;
                int knobY = trackY - knobD / 2;
                using (var bKnob = new SolidBrush(_draggingSens || _hoverCtrl == 7 ? TextPrimary : AccentCyan))
                using (var pKnob = new Pen(BgWindow, 1.5f))
                {
                    g.FillEllipse(bKnob, knobX, knobY, knobD, knobD);
                    g.DrawEllipse(pKnob, knobX, knobY, knobD, knobD);
                }

                string sensText = _mouseSens.ToString("0.0", CultureInfo.InvariantCulture) + "x";
                g.DrawString(sensText, fVal, bAccent, _sensTrackRect.Right + S(12), _sensTrackRect.Y + S(1));

                // Row 3: Mouse button mapping pills
                g.DrawString("Botões Mouse:", fLbl, bMuted, _mousePanelRect.X + S(14), _btnRmbRect.Y + S(4));
                DrawSegmentPill(g, _btnRmbRect, "Dir: Atirar | Esq: Mirar", _mouseButtons == "rmb_shoot", _hoverCtrl == 8);
                DrawSegmentPill(g, _btnLmbRect, "Esq: Atirar | Dir: Mirar", _mouseButtons == "lmb_shoot", _hoverCtrl == 9);
                DrawSegmentPill(g, _btnOffRect, "Desativado", _mouseButtons == "off", _hoverCtrl == 10);

                // Row 4: Xbox -> Keyboard remapper button + active summary
                string customSpec = SerializeCustomKeymap();
                bool hasCustom = !string.IsNullOrEmpty(customSpec);
                DrawSegmentPill(g, _keymapBtnRect, "⌨  Mapear Botões Xbox → Teclado", hasCustom, _hoverCtrl == 12);

                string summary = hasCustom ? ("Ativo: " + customSpec) : "Padrão (WASD, Enter=A, Back=B, X, Y, Q/E, Z/C)";
                if (summary.Length > 52) summary = summary.Substring(0, 49) + "...";
                using (var bSum = new SolidBrush(hasCustom ? AccentGreen : TextMuted))
                {
                    g.DrawString(summary, fLbl, bSum, _keymapBtnRect.Right + S(12), _keymapBtnRect.Y + S(5));
                }
            }
        }

        private void DrawSegmentPill(Graphics g, Rectangle r, string label, bool active, bool hover)
        {
            Color bg = active ? Color.FromArgb(38, AccentCyan) : (hover ? BgCardHover : BgCard);
            Color border = active ? AccentCyan : (hover ? Color.FromArgb(72, 86, 114) : BorderSubtle);
            Color txt = active ? AccentCyan : (hover ? TextPrimary : TextMuted);

            using (var path = RoundedRect(r, S(6)))
            using (var bBg = new SolidBrush(bg))
            using (var pPen = new Pen(border, active ? 1.2f : 1f))
            using (var f = new Font("Segoe UI Semibold", 7.8f, FontStyle.Bold))
            using (var bTxt = new SolidBrush(txt))
            using (var sf = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center })
            {
                g.FillPath(bBg, path);
                g.DrawPath(pPen, path);
                g.DrawString(label, f, bTxt, r, sf);
            }
        }

        private void DrawCheckbox(Graphics g, Rectangle r, bool checkedState, string label, bool hover)
        {
            Rectangle box = new Rectangle(r.X, r.Y + S(3), S(15), S(15));
            using (var bp = RoundedRect(box, S(4)))
            using (var bFill = new SolidBrush(checkedState ? AccentCyan : BgCard))
            using (var pPen = new Pen(checkedState ? AccentCyan : (hover ? TextMuted : BorderSubtle), 1.2f))
            {
                g.FillPath(bFill, bp);
                g.DrawPath(pPen, bp);
            }

            if (checkedState)
            {
                using (var pChk = new Pen(BgWindow, 2f))
                {
                    g.DrawLine(pChk, box.X + S(3), box.Y + S(7), box.X + S(6), box.Y + S(11));
                    g.DrawLine(pChk, box.X + S(6), box.Y + S(11), box.X + S(12), box.Y + S(4));
                }
            }

            using (var f = new Font("Segoe UI", 8.6f))
            using (var b = new SolidBrush(hover || checkedState ? TextPrimary : TextMuted))
            {
                g.DrawString(label, f, b, box.Right + S(8), r.Y + S(2));
            }
        }

        private void DrawButton(Graphics g, Rectangle r, string text, bool primary, bool hover)
        {
            using (var path = RoundedRect(r, S(9)))
            {
                if (primary)
                {
                    Color c1 = hover ? Color.FromArgb(255, 86, 164) : AccentPink;
                    Color c2 = hover ? Color.FromArgb(56, 228, 250) : AccentCyan;
                    using (var grad = new LinearGradientBrush(r, c1, c2, 0f))
                    {
                        g.FillPath(grad, path);
                    }
                }
                else
                {
                    using (var bBg = new SolidBrush(hover ? BgCardHover : BgCard))
                    using (var pPen = new Pen(hover ? Color.FromArgb(72, 86, 114) : BorderSubtle, 1f))
                    {
                        g.FillPath(bBg, path);
                        g.DrawPath(pPen, path);
                    }
                }
            }

            using (var f = new Font("Segoe UI Semibold", primary ? 9.6f : 8.6f, FontStyle.Bold))
            using (var bTxt = new SolidBrush(primary ? Color.FromArgb(10, 12, 18) : TextPrimary))
            using (var sf = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center })
            {
                g.DrawString(text, f, bTxt, r, sf);
            }
        }

        private void UpdateSensFromX(int mouseX)
        {
            float norm = (float)(mouseX - _sensTrackRect.X) / Math.Max(1, _sensTrackRect.Width);
            norm = Math.Max(0f, Math.Min(1f, norm));
            float val = 0.2f + norm * (4.0f - 0.2f);
            _mouseSens = (float)Math.Round(val * 10f) / 10f;
            Invalidate();
        }

        private void OnMouseMoveCanvas(object sender, MouseEventArgs e)
        {
            if (_draggingSens)
            {
                UpdateSensFromX(e.X);
                return;
            }

            int oldIdx = _hoverIndex;
            int oldCtrl = _hoverCtrl;
            _hoverIndex = -1;
            _hoverCtrl = -1;

            for (int i = 0; i < _cardRects.Length; i++)
            {
                if (_cardRects[i].Contains(e.Location))
                {
                    _hoverIndex = i;
                    break;
                }
            }

            if (_playRect.Contains(e.Location)) _hoverCtrl = 0;
            else if (_precompileRect.Contains(e.Location)) _hoverCtrl = 1;
            else if (_logsRect.Contains(e.Location)) _hoverCtrl = 2;
            else if (_hudChkRect.Contains(e.Location)) _hoverCtrl = 3;
            else if (_closeChkRect.Contains(e.Location)) _hoverCtrl = 4;
            else if (_mouseCamChkRect.Contains(e.Location)) _hoverCtrl = 5;
            else if (_invertYChkRect.Contains(e.Location)) _hoverCtrl = 6;
            else if (_sensTrackRect.Contains(e.Location)) _hoverCtrl = 7;
            else if (_btnRmbRect.Contains(e.Location)) _hoverCtrl = 8;
            else if (_btnLmbRect.Contains(e.Location)) _hoverCtrl = 9;
            else if (_btnOffRect.Contains(e.Location)) _hoverCtrl = 10;
            else if (_unifiedChkRect.Contains(e.Location)) _hoverCtrl = 11;
            else if (_keymapBtnRect.Contains(e.Location)) _hoverCtrl = 12;
            else if (_liteChkRect.Contains(e.Location)) _hoverCtrl = 13;

            Cursor = (_hoverIndex >= 0 || _hoverCtrl >= 0) ? Cursors.Hand : Cursors.Default;
            if (oldIdx != _hoverIndex || oldCtrl != _hoverCtrl)
                Invalidate();
        }

        private void OnMouseDownCanvas(object sender, MouseEventArgs e)
        {
            if (e.Button != MouseButtons.Left) return;

            for (int i = 0; i < _cardRects.Length; i++)
            {
                if (_cardRects[i].Contains(e.Location))
                {
                    _selectedIndex = i;
                    _statusText = "";
                    SaveConfig();
                    Invalidate();
                    return;
                }
            }

            if (_mouseCamChkRect.Contains(e.Location))
            {
                _mouseCamera = !_mouseCamera;
                SaveConfig();
                Invalidate();
            }
            else if (_invertYChkRect.Contains(e.Location))
            {
                _mouseInvertY = !_mouseInvertY;
                SaveConfig();
                Invalidate();
            }
            else if (_sensTrackRect.Contains(e.Location))
            {
                _draggingSens = true;
                UpdateSensFromX(e.X);
            }
            else if (_btnRmbRect.Contains(e.Location))
            {
                _mouseButtons = "rmb_shoot";
                SaveConfig();
                Invalidate();
            }
            else if (_btnLmbRect.Contains(e.Location))
            {
                _mouseButtons = "lmb_shoot";
                SaveConfig();
                Invalidate();
            }
            else if (_btnOffRect.Contains(e.Location))
            {
                _mouseButtons = "off";
                SaveConfig();
                Invalidate();
            }
            else if (_keymapBtnRect.Contains(e.Location))
            {
                OpenKeymapDialog();
            }
            else if (_unifiedChkRect.Contains(e.Location))
            {
                _unifiedMemory = !_unifiedMemory;
                SaveConfig();
                Invalidate();
            }
            else if (_liteChkRect.Contains(e.Location))
            {
                _liteGraphics = !_liteGraphics;
                SaveConfig();
                Invalidate();
            }
            else if (_hudChkRect.Contains(e.Location))
            {
                _enableHud = !_enableHud;
                SaveConfig();
                Invalidate();
            }
            else if (_closeChkRect.Contains(e.Location))
            {
                _closeOnLaunch = !_closeOnLaunch;
                SaveConfig();
                Invalidate();
            }
            else if (_playRect.Contains(e.Location))
            {
                LaunchSelected();
            }
            else if (_precompileRect.Contains(e.Location))
            {
                LaunchFile(PrecompileBat, "");
            }
            else if (_logsRect.Contains(e.Location))
            {
                try { Process.Start("explorer.exe", DataDir); } catch { }
            }
        }

        private void OpenKeymapDialog()
        {
            var copy = new List<KeyBindingItem>(_keymap.Count);
            foreach (var it in _keymap)
            {
                copy.Add(new KeyBindingItem
                {
                    Id = it.Id,
                    Label = it.Label,
                    DefaultKey = it.DefaultKey,
                    Key = it.Key,
                    Accent = it.Accent
                });
            }
            using (var dlg = new KeymapForm(copy, _scale))
            {
                if (dlg.ShowDialog(this) == DialogResult.OK)
                {
                    for (int i = 0; i < _keymap.Count; i++)
                    {
                        _keymap[i].Key = copy[i].Key;
                    }
                    SaveConfig();
                    _statusText = "Mapeamento de controles salvo.";
                    Invalidate();
                }
            }
        }

        private void OnMouseDoubleClickCanvas(object sender, MouseEventArgs e)
        {
            if (e.Button != MouseButtons.Left) return;
            for (int i = 0; i < _cardRects.Length; i++)
            {
                if (_cardRects[i].Contains(e.Location))
                {
                    _selectedIndex = i;
                    SaveConfig();
                    LaunchSelected();
                    return;
                }
            }
        }

        private void OnKeyDownCanvas(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Up && _selectedIndex > 0)
            {
                _selectedIndex--;
                SaveConfig();
                Invalidate();
            }
            else if (e.KeyCode == Keys.Down && _selectedIndex < _versions.Count - 1)
            {
                _selectedIndex++;
                SaveConfig();
                Invalidate();
            }
            else if (e.KeyCode == Keys.Enter)
            {
                LaunchSelected();
            }
            else if (e.KeyCode == Keys.Escape)
            {
                Close();
            }
        }

        private void LaunchSelected()
        {
            if (_selectedIndex < 0 || _selectedIndex >= _versions.Count) return;
            GameVersion v = _versions[_selectedIndex];
            if (!v.Available)
            {
                _statusText = "Build ainda não disponível em disco.";
                Invalidate();
                return;
            }

            SaveConfig();
            try
            {
                if (v.Kind == "ps1")
                {
                    var parts = new List<string>();
                    if (!string.IsNullOrEmpty(v.Args)) parts.Add(v.Args);
                    if (v.SupportsOptions)
                    {
                        if (!_enableHud) parts.Add("-NoHud");
                        if (!_unifiedMemory) parts.Add("-NoUnifiedMemory");
                        if (_liteGraphics) parts.Add("-LiteGraphics");
                        if (_mouseCamera)
                        {
                            parts.Add("-MouseCamera");
                            parts.Add("-MouseSens " + _mouseSens.ToString("0.00", CultureInfo.InvariantCulture));
                            if (_mouseInvertY) parts.Add("-MouseInvertY");
                        }
                        if (_mouseButtons != "off")
                        {
                            parts.Add("-MouseButtons " + _mouseButtons);
                        }
                        string km = SerializeCustomKeymap();
                        if (!string.IsNullOrEmpty(km))
                        {
                            parts.Add("-Keymap \"" + km + "\"");
                        }
                    }
                    string extra = string.Join(" ", parts.ToArray());

                    var psi = new ProcessStartInfo
                    {
                        FileName = "powershell.exe",
                        Arguments = "-NoProfile -ExecutionPolicy Bypass -File \"" + v.Target + "\"" + (string.IsNullOrEmpty(extra) ? "" : " " + extra),
                        WorkingDirectory = RepoDir,
                        UseShellExecute = true
                    };
                    Process.Start(psi);
                }
                else
                {
                    LaunchFile(v.Target, v.Args);
                }

                if (_closeOnLaunch)
                {
                    Close();
                }
                else
                {
                    _statusText = "Iniciado: " + v.Title;
                    TrimMemory();
                    Invalidate();
                }
            }
            catch (Exception ex)
            {
                _statusText = "Erro ao iniciar: " + ex.Message;
                Invalidate();
            }
        }

        private void LaunchFile(string path, string args)
        {
            if (!File.Exists(path))
            {
                _statusText = "Arquivo não encontrado: " + Path.GetFileName(path);
                Invalidate();
                return;
            }
            var psi = new ProcessStartInfo
            {
                FileName = path,
                Arguments = args,
                WorkingDirectory = Path.GetDirectoryName(path),
                UseShellExecute = true
            };
            Process.Start(psi);
        }

        [STAThread]
        static void Main()
        {
            try { SetProcessDPIAware(); } catch { }
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            Application.Run(new LauncherForm());
        }
    }
}
