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
    sealed class BuildEntry
    {
        public string Name;
        public string Meta;
        public string ExePath;
        public bool Available;
    }

    sealed class PresetEntry
    {
        public string TitlePt;
        public string TitleEn;
        public string SubPt;
        public string SubEn;
        public string TagPt;
        public string TagEn;
        public Color TagColor;
        public string Args;
    }

    sealed class KeyBindingItem
    {
        public string Id;
        public string LabelPt;
        public string LabelEn;
        public string DefaultKey;
        public string Key;
        public Color Accent;
    }

    static class L10n
    {
        public static bool IsEn = false;
        public static string T(string pt, string en) { return IsEn ? en : pt; }
    }

    static class UiTheme
    {
        public static readonly Color Bg = Color.FromArgb(12, 14, 19);
        public static readonly Color Surface = Color.FromArgb(19, 22, 30);
        public static readonly Color SurfaceHover = Color.FromArgb(26, 31, 42);
        public static readonly Color SurfaceActive = Color.FromArgb(26, 35, 50);
        public static readonly Color SurfaceDisabled = Color.FromArgb(16, 18, 24);
        public static readonly Color Border = Color.FromArgb(34, 41, 56);
        public static readonly Color BorderHover = Color.FromArgb(58, 70, 95);
        public static readonly Color Cyan = Color.FromArgb(34, 211, 238);
        public static readonly Color Pink = Color.FromArgb(244, 63, 148);
        public static readonly Color Green = Color.FromArgb(52, 211, 153);
        public static readonly Color Amber = Color.FromArgb(251, 191, 36);
        public static readonly Color Blurple = Color.FromArgb(114, 137, 218);
        public static readonly Color Red = Color.FromArgb(248, 113, 113);
        public static readonly Color Blue = Color.FromArgb(96, 165, 250);
        public static readonly Color Slate = Color.FromArgb(148, 163, 184);
        public static readonly Color Text = Color.FromArgb(241, 245, 249);
        public static readonly Color Muted = Color.FromArgb(132, 146, 168);
        public static readonly Color Dim = Color.FromArgb(72, 82, 100);

        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int attrValue, int attrSize);

        public static void ApplyDarkFrame(IntPtr hwnd)
        {
            try
            {
                int dark = 1;
                DwmSetWindowAttribute(hwnd, 20, ref dark, sizeof(int));
                int corner = 2;
                DwmSetWindowAttribute(hwnd, 33, ref corner, sizeof(int));
            }
            catch { }
        }

        public static GraphicsPath RoundedRect(Rectangle r, int radius)
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

        /// Draws crisp vector Brazil flag inside `r`.
        public static void DrawBrazilFlag(Graphics g, Rectangle r)
        {
            var oldClip = g.Clip;
            using (var clipPath = RoundedRect(r, 2))
            {
                g.SetClip(clipPath, CombineMode.Intersect);
                using (var bGreen = new SolidBrush(Color.FromArgb(0, 156, 59)))
                    g.FillRectangle(bGreen, r);

                PointF[] diamond = new PointF[]
                {
                    new PointF(r.X + r.Width * 0.50f, r.Y + r.Height * 0.14f),
                    new PointF(r.X + r.Width * 0.88f, r.Y + r.Height * 0.50f),
                    new PointF(r.X + r.Width * 0.50f, r.Y + r.Height * 0.86f),
                    new PointF(r.X + r.Width * 0.12f, r.Y + r.Height * 0.50f)
                };
                using (var bYellow = new SolidBrush(Color.FromArgb(255, 223, 0)))
                    g.FillPolygon(bYellow, diamond);

                float d = r.Height * 0.44f;
                float cx = r.X + (r.Width - d) * 0.5f;
                float cy = r.Y + (r.Height - d) * 0.5f;
                using (var bBlue = new SolidBrush(Color.FromArgb(0, 39, 118)))
                    g.FillEllipse(bBlue, cx, cy, d, d);

                using (var pWhite = new Pen(Color.White, Math.Max(1f, r.Height * 0.08f)))
                    g.DrawLine(pWhite, cx + d * 0.08f, cy + d * 0.44f, cx + d * 0.92f, cy + d * 0.60f);
            }
            g.Clip = oldClip;
        }

        /// Draws crisp vector US flag inside `r`.
        public static void DrawUsFlag(Graphics g, Rectangle r)
        {
            var oldClip = g.Clip;
            using (var clipPath = RoundedRect(r, 2))
            {
                g.SetClip(clipPath, CombineMode.Intersect);
                using (var bWhite = new SolidBrush(Color.White))
                    g.FillRectangle(bWhite, r);

                float stripeH = r.Height / 7f;
                using (var bRed = new SolidBrush(Color.FromArgb(191, 10, 48)))
                {
                    for (int i = 0; i < 7; i += 2)
                        g.FillRectangle(bRed, r.X, r.Y + i * stripeH, r.Width, stripeH + 0.5f);
                }

                RectangleF canton = new RectangleF(r.X, r.Y, r.Width * 0.46f, r.Height * 0.56f);
                using (var bBlue = new SolidBrush(Color.FromArgb(0, 40, 104)))
                    g.FillRectangle(bBlue, canton);

                using (var bStar = new SolidBrush(Color.White))
                {
                    float dot = Math.Max(1.1f, r.Height * 0.09f);
                    for (int row = 0; row < 3; row++)
                    {
                        for (int col = 0; col < 3; col++)
                        {
                            float sx = canton.X + canton.Width * (0.20f + col * 0.30f) - dot * 0.5f;
                            float sy = canton.Y + canton.Height * (0.22f + row * 0.28f) - dot * 0.5f;
                            g.FillEllipse(bStar, sx, sy, dot, dot);
                        }
                    }
                }
            }
            g.Clip = oldClip;
        }
    }

    sealed class DiscordConfigForm : Form
    {
        public string AppIdValue { get; private set; }
        public bool HideTitleValue { get; private set; }

        private readonly TextBox _tbId;
        private readonly CheckBox _cbHide;

        public DiscordConfigForm(string currentId, bool hideTitle)
        {
            AppIdValue = currentId ?? "";
            HideTitleValue = hideTitle;

            Text = "Discord Rich Presence";
            StartPosition = FormStartPosition.CenterParent;
            FormBorderStyle = FormBorderStyle.FixedDialog;
            MaximizeBox = false;
            MinimizeBox = false;
            BackColor = UiTheme.Bg;
            ForeColor = UiTheme.Text;
            ClientSize = new Size(420, 215);
            Font = new Font("Segoe UI", 9f);

            var lblTitle = new Label
            {
                Text = "DISCORD RICH PRESENCE (FLAMINGO)",
                Font = new Font("Segoe UI Semibold", 9.5f, FontStyle.Bold),
                ForeColor = UiTheme.Blurple,
                Location = new Point(20, 16),
                AutoSize = true
            };

            var lblHint = new Label
            {
                Text = L10n.T("Cole o Application ID do Discord Developer Portal (somente dígitos):", "Paste your Discord Developer Portal Application ID (digits only):"),
                ForeColor = UiTheme.Muted,
                Location = new Point(20, 42),
                Size = new Size(380, 20)
            };

            _tbId = new TextBox
            {
                Text = AppIdValue,
                BackColor = UiTheme.Surface,
                ForeColor = UiTheme.Text,
                BorderStyle = BorderStyle.FixedSingle,
                Font = new Font("Consolas", 10.5f),
                Location = new Point(20, 68),
                Size = new Size(380, 26)
            };

            _cbHide = new CheckBox
            {
                Text = L10n.T("Ocultar nome do jogo no status (mostrar apenas FPS e tempo)", "Hide game title in status (show FPS and elapsed time only)"),
                Checked = hideTitle,
                ForeColor = UiTheme.Text,
                BackColor = UiTheme.Bg,
                Location = new Point(20, 106),
                Size = new Size(380, 24)
            };

            var lblPortal = new LinkLabel
            {
                Text = L10n.T("Abrir Discord Developer Portal", "Open Discord Developer Portal"),
                LinkColor = UiTheme.Cyan,
                ActiveLinkColor = UiTheme.Pink,
                Location = new Point(20, 134),
                AutoSize = true
            };
            lblPortal.LinkClicked += (s, e) =>
            {
                try { Process.Start("https://discord.com/developers/applications"); } catch { }
            };

            var btnCancel = new Button
            {
                Text = L10n.T("Cancelar", "Cancel"),
                FlatStyle = FlatStyle.Flat,
                BackColor = UiTheme.Surface,
                ForeColor = UiTheme.Text,
                Location = new Point(214, 166),
                Size = new Size(88, 32),
                DialogResult = DialogResult.Cancel
            };
            btnCancel.FlatAppearance.BorderColor = UiTheme.Border;

            var btnSave = new Button
            {
                Text = L10n.T("Salvar", "Save"),
                FlatStyle = FlatStyle.Flat,
                BackColor = UiTheme.Blurple,
                ForeColor = Color.White,
                Font = new Font("Segoe UI Semibold", 9f, FontStyle.Bold),
                Location = new Point(310, 166),
                Size = new Size(90, 32)
            };
            btnSave.FlatAppearance.BorderSize = 0;
            btnSave.Click += (s, e) =>
            {
                string cleaned = _tbId.Text.Trim();
                if (cleaned.Length > 0)
                {
                    foreach (char c in cleaned)
                    {
                        if (c < '0' || c > '9')
                        {
                            MessageBox.Show(
                                this,
                                L10n.T("O Application ID deve conter apenas números.", "Application ID must contain digits only."),
                                "Discord RPC",
                                MessageBoxButtons.OK,
                                MessageBoxIcon.Warning);
                            return;
                        }
                    }
                }
                AppIdValue = cleaned;
                HideTitleValue = _cbHide.Checked;
                DialogResult = DialogResult.OK;
                Close();
            };

            Controls.AddRange(new Control[] { lblTitle, lblHint, _tbId, _cbHide, lblPortal, btnCancel, btnSave });
            AcceptButton = btnSave;
            CancelButton = btnCancel;
            Shown += (s, e) => UiTheme.ApplyDarkFrame(Handle);
        }
    }

    sealed class KeymapForm : Form
    {
        private readonly List<KeyBindingItem> _items;
        private readonly float _scale;
        private Rectangle[] _rowRects;
        private Rectangle _resetRect;
        private Rectangle _cancelRect;
        private Rectangle _saveRect;
        private int _hoverRow = -1;
        private int _hoverBtn = -1;
        private int _listeningIndex = -1;
        private string _hint;

        private int S(int v) { return (int)Math.Round(v * _scale); }

        public KeymapForm(List<KeyBindingItem> items, float scale)
        {
            _items = items;
            _scale = scale;
            _hint = L10n.T("Clique em um botão do Xbox e pressione a tecla desejada (ESC cancela • DEL desativa).", "Click an Xbox button and press the desired key (ESC cancels • DEL disables).");
            Text = L10n.T("Mapear Controles — Xbox → Teclado", "Map Controls — Xbox → Keyboard");
            StartPosition = FormStartPosition.CenterParent;
            FormBorderStyle = FormBorderStyle.FixedDialog;
            MaximizeBox = false;
            MinimizeBox = false;
            BackColor = UiTheme.Bg;
            ForeColor = UiTheme.Text;
            DoubleBuffered = true;
            KeyPreview = true;
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.OptimizedDoubleBuffer, true);

            int rowsPerCol = (_items.Count + 1) / 2;
            ClientSize = new Size(S(620), S(72) + rowsPerCol * S(30) + S(80));
            ComputeLayout();

            MouseMove += OnMouseMoveCanvas;
            MouseDown += OnMouseDownCanvas;
            KeyDown += OnKeyDownCanvas;
            Shown += (s, e) => UiTheme.ApplyDarkFrame(Handle);
        }

        private void ComputeLayout()
        {
            _rowRects = new Rectangle[_items.Count];
            int pad = S(18);
            int gap = S(12);
            int colW = (ClientSize.Width - pad * 2 - gap) / 2;
            int rowsPerCol = (_items.Count + 1) / 2;
            int startY = S(58);
            for (int i = 0; i < _items.Count; i++)
            {
                int col = i / rowsPerCol;
                int row = i % rowsPerCol;
                _rowRects[i] = new Rectangle(pad + col * (colW + gap), startY + row * S(30), colW, S(25));
            }
            int btnY = startY + rowsPerCol * S(30) + S(32);
            _resetRect = new Rectangle(pad, btnY, S(150), S(32));
            _cancelRect = new Rectangle(ClientSize.Width - pad - S(248), btnY, S(108), S(32));
            _saveRect = new Rectangle(ClientSize.Width - pad - S(132), btnY, S(132), S(32));
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            Graphics g = e.Graphics;
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

            int pad = S(18);
            using (var fTitle = new Font("Segoe UI Semibold", 11f, FontStyle.Bold))
            using (var fSub = new Font("Segoe UI", 8.2f))
            using (var fRow = new Font("Segoe UI", 8.4f))
            using (var fKey = new Font("Segoe UI Semibold", 8f, FontStyle.Bold))
            using (var bPri = new SolidBrush(UiTheme.Text))
            using (var bMut = new SolidBrush(UiTheme.Muted))
            using (var sfC = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center })
            {
                g.DrawString(L10n.T("MAPEAMENTO XBOX → TECLADO", "XBOX → KEYBOARD MAPPING"), fTitle, bPri, pad, S(14));
                g.DrawString(L10n.T("Clique em qualquer ação para redefinir a tecla", "Click any action to rebind its key"), fSub, bMut, pad, S(36));

                for (int i = 0; i < _items.Count; i++)
                {
                    var item = _items[i];
                    Rectangle r = _rowRects[i];
                    bool listening = (i == _listeningIndex);
                    bool hover = (i == _hoverRow);

                    Color bg = listening ? Color.FromArgb(40, UiTheme.Pink) : (hover ? UiTheme.SurfaceHover : UiTheme.Surface);
                    Color border = listening ? UiTheme.Pink : (hover ? UiTheme.Cyan : UiTheme.Border);
                    using (var p = UiTheme.RoundedRect(r, S(5)))
                    using (var bBg = new SolidBrush(bg))
                    using (var pen = new Pen(border, listening ? 1.3f : 1f))
                    {
                        g.FillPath(bBg, p);
                        g.DrawPath(pen, p);
                    }
                    using (var bDot = new SolidBrush(item.Accent))
                        g.FillEllipse(bDot, r.X + S(8), r.Y + S(8), S(7), S(7));
                    g.DrawString(L10n.IsEn ? item.LabelEn : item.LabelPt, fRow, bPri, r.X + S(21), r.Y + S(4));

                    string keyText = listening ? L10n.T("TECLA...", "KEY...") : (string.IsNullOrEmpty(item.Key) || item.Key == "NONE" ? "OFF" : item.Key);
                    bool modified = !string.Equals(item.Key, item.DefaultKey, StringComparison.OrdinalIgnoreCase);
                    Color kColor = listening ? UiTheme.Pink : (modified ? UiTheme.Cyan : UiTheme.Green);
                    Rectangle kr = new Rectangle(r.Right - S(94), r.Y + S(3), S(89), r.Height - S(6));
                    using (var kp = UiTheme.RoundedRect(kr, S(4)))
                    using (var bKBg = new SolidBrush(Color.FromArgb(28, kColor)))
                    using (var bKTxt = new SolidBrush(kColor))
                    {
                        g.FillPath(bKBg, kp);
                        g.DrawString(keyText, fKey, bKTxt, kr, sfC);
                    }
                }

                int rowsPerCol = (_items.Count + 1) / 2;
                int hintY = S(58) + rowsPerCol * S(30) + S(8);
                using (var bHint = new SolidBrush(_listeningIndex >= 0 ? UiTheme.Pink : UiTheme.Amber))
                    g.DrawString(_hint, fSub, bHint, pad, hintY);

                DrawBtn(g, _resetRect, L10n.T("Restaurar Padrão", "Reset Defaults"), false, _hoverBtn == 1, sfC);
                DrawBtn(g, _cancelRect, L10n.T("Cancelar", "Cancel"), false, _hoverBtn == 2, sfC);
                DrawBtn(g, _saveRect, L10n.T("Salvar", "Save"), true, _hoverBtn == 0, sfC);
            }
        }

        private void DrawBtn(Graphics g, Rectangle r, string text, bool primary, bool hover, StringFormat sf)
        {
            using (var path = UiTheme.RoundedRect(r, S(7)))
            {
                if (primary)
                {
                    using (var grad = new LinearGradientBrush(r, UiTheme.Pink, UiTheme.Cyan, 0f))
                        g.FillPath(grad, path);
                }
                else
                {
                    using (var bBg = new SolidBrush(hover ? UiTheme.SurfaceHover : UiTheme.Surface))
                    using (var pen = new Pen(hover ? UiTheme.BorderHover : UiTheme.Border, 1f))
                    {
                        g.FillPath(bBg, path);
                        g.DrawPath(pen, path);
                    }
                }
            }
            using (var f = new Font("Segoe UI Semibold", 8.6f, FontStyle.Bold))
            using (var b = new SolidBrush(primary ? Color.FromArgb(10, 12, 18) : UiTheme.Text))
                g.DrawString(text, f, b, r, sf);
        }

        private void OnMouseMoveCanvas(object sender, MouseEventArgs e)
        {
            int oldRow = _hoverRow, oldBtn = _hoverBtn;
            _hoverRow = -1; _hoverBtn = -1;
            for (int i = 0; i < _rowRects.Length; i++)
                if (_rowRects[i].Contains(e.Location)) { _hoverRow = i; break; }
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
                    string lbl = L10n.IsEn ? _items[i].LabelEn : _items[i].LabelPt;
                    _hint = _listeningIndex >= 0
                        ? L10n.T("Pressione uma tecla para \"" + lbl + "\" (ESC cancela • DEL desativa)", "Press a key for \"" + lbl + "\" (ESC cancels • DEL disables)")
                        : L10n.T("Clique em um botão do Xbox e pressione a tecla desejada.", "Click an Xbox button and press the desired key.");
                    Invalidate();
                    return;
                }
            }
            if (_resetRect.Contains(e.Location))
            {
                _listeningIndex = -1;
                foreach (var it in _items) it.Key = it.DefaultKey;
                _hint = L10n.T("Padrões restaurados.", "Defaults restored.");
                Invalidate();
            }
            else if (_cancelRect.Contains(e.Location)) { DialogResult = DialogResult.Cancel; Close(); }
            else if (_saveRect.Contains(e.Location)) { DialogResult = DialogResult.OK; Close(); }
        }

        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            if (_listeningIndex >= 0) { HandleCapturedKey(keyData & Keys.KeyCode); return true; }
            return base.ProcessCmdKey(ref msg, keyData);
        }

        private void OnKeyDownCanvas(object sender, KeyEventArgs e)
        {
            if (_listeningIndex >= 0) { HandleCapturedKey(e.KeyCode); e.Handled = true; e.SuppressKeyPress = true; return; }
            if (e.KeyCode == Keys.Escape) { DialogResult = DialogResult.Cancel; Close(); }
        }

        private void HandleCapturedKey(Keys code)
        {
            if (_listeningIndex < 0) return;
            if (code == Keys.Escape) { _listeningIndex = -1; _hint = L10n.T("Cancelado.", "Canceled."); Invalidate(); return; }
            string mapped = code == Keys.Delete ? "NONE" : KeyToSpec(code);
            if (string.IsNullOrEmpty(mapped)) { _hint = L10n.T("Tecla não suportada.", "Unsupported key."); Invalidate(); return; }
            var target = _items[_listeningIndex];
            if (mapped != "NONE")
                foreach (var other in _items)
                    if (other != target && string.Equals(other.Key, mapped, StringComparison.OrdinalIgnoreCase))
                        other.Key = "NONE";
            target.Key = mapped;
            _listeningIndex = -1;
            _hint = (L10n.IsEn ? target.LabelEn : target.LabelPt) + " = " + mapped;
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
                case Keys.ShiftKey: case Keys.LShiftKey: case Keys.RShiftKey: return "SHIFT";
                case Keys.ControlKey: case Keys.LControlKey: case Keys.RControlKey: return "CTRL";
                case Keys.Menu: case Keys.LMenu: case Keys.RMenu: return "ALT";
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
                default:
                    int vk = (int)code;
                    return (vk >= 8 && vk <= 254) ? ("0x" + vk.ToString("X2")) : null;
            }
        }
    }

    sealed class ControlsForm : Form
    {
        public bool MouseCamera { get; private set; }
        public bool MouseInvertY { get; private set; }
        public float MouseSens { get; private set; }
        public string MouseButtonsMode { get; private set; }
        private readonly List<KeyBindingItem> _keymap;
        private readonly float _scale;

        private readonly CheckBox _cbCam;
        private readonly CheckBox _cbInv;
        private readonly TrackBar _tbSens;
        private readonly Label _lblSensVal;
        private readonly ComboBox _cmbBtns;

        public ControlsForm(bool cam, bool inv, float sens, string btns, List<KeyBindingItem> keymap, float scale)
        {
            MouseCamera = cam;
            MouseInvertY = inv;
            MouseSens = sens;
            MouseButtonsMode = btns;
            _keymap = keymap;
            _scale = scale;

            Text = L10n.T("Controles — Mouse & Teclado", "Controls — Mouse & Keyboard");
            StartPosition = FormStartPosition.CenterParent;
            FormBorderStyle = FormBorderStyle.FixedDialog;
            MaximizeBox = false;
            MinimizeBox = false;
            BackColor = UiTheme.Bg;
            ForeColor = UiTheme.Text;
            ClientSize = new Size(420, 260);
            Font = new Font("Segoe UI", 9f);

            var lblHdr = new Label
            {
                Text = L10n.T("MOUSE E MAPEAMENTO XBOX → TECLADO", "MOUSE & XBOX → KEYBOARD MAPPING"),
                Font = new Font("Segoe UI Semibold", 9.2f, FontStyle.Bold),
                ForeColor = UiTheme.Cyan,
                Location = new Point(20, 16),
                AutoSize = true
            };

            _cbCam = new CheckBox
            {
                Text = L10n.T("Mouse como Câmera (Analógico Direito • F9 trava/solta)", "Mouse as Camera (Right Stick • F9 locks/unlocks)"),
                Checked = cam,
                ForeColor = UiTheme.Text,
                Location = new Point(20, 46),
                Size = new Size(380, 24)
            };

            _cbInv = new CheckBox
            {
                Text = L10n.T("Inverter Eixo Y", "Invert Y Axis"),
                Checked = inv,
                ForeColor = UiTheme.Muted,
                Location = new Point(20, 72),
                Size = new Size(160, 24)
            };

            var lblSens = new Label { Text = L10n.T("Sensibilidade:", "Sensitivity:"), ForeColor = UiTheme.Muted, Location = new Point(20, 108), AutoSize = true };
            _lblSensVal = new Label { Text = sens.ToString("0.0", CultureInfo.InvariantCulture) + "x", ForeColor = UiTheme.Cyan, Font = new Font("Segoe UI Semibold", 9f, FontStyle.Bold), Location = new Point(358, 108), AutoSize = true };
            _tbSens = new TrackBar
            {
                Minimum = 2,
                Maximum = 40,
                Value = Math.Max(2, Math.Min(40, (int)Math.Round(sens * 10))),
                TickStyle = TickStyle.None,
                BackColor = UiTheme.Bg,
                Location = new Point(112, 102),
                Size = new Size(238, 30)
            };
            _tbSens.ValueChanged += (s, e) =>
            {
                _lblSensVal.Text = (_tbSens.Value / 10f).ToString("0.0", CultureInfo.InvariantCulture) + "x";
            };

            var lblBtns = new Label { Text = L10n.T("Botões Mouse:", "Mouse Buttons:"), ForeColor = UiTheme.Muted, Location = new Point(20, 146), AutoSize = true };
            _cmbBtns = new ComboBox
            {
                DropDownStyle = ComboBoxStyle.DropDownList,
                BackColor = UiTheme.Surface,
                ForeColor = UiTheme.Text,
                FlatStyle = FlatStyle.Flat,
                Location = new Point(118, 142),
                Size = new Size(282, 24)
            };
            _cmbBtns.Items.AddRange(new object[]
            {
                L10n.T("Dir: Atirar | Esq: Mirar", "Right: Shoot | Left: Aim"),
                L10n.T("Esq: Atirar | Dir: Mirar", "Left: Shoot | Right: Aim"),
                L10n.T("Desativado", "Disabled")
            });
            _cmbBtns.SelectedIndex = btns == "lmb_shoot" ? 1 : (btns == "off" ? 2 : 0);

            var btnKeymap = new Button
            {
                Text = L10n.T("Mapear Botões Xbox → Teclado...", "Map Xbox → Keyboard Buttons..."),
                FlatStyle = FlatStyle.Flat,
                BackColor = UiTheme.Surface,
                ForeColor = UiTheme.Cyan,
                Location = new Point(20, 180),
                Size = new Size(380, 30)
            };
            btnKeymap.FlatAppearance.BorderColor = UiTheme.Border;
            btnKeymap.Click += (s, e) =>
            {
                var copy = new List<KeyBindingItem>(_keymap.Count);
                foreach (var it in _keymap)
                    copy.Add(new KeyBindingItem { Id = it.Id, LabelPt = it.LabelPt, LabelEn = it.LabelEn, DefaultKey = it.DefaultKey, Key = it.Key, Accent = it.Accent });
                using (var dlg = new KeymapForm(copy, _scale))
                {
                    if (dlg.ShowDialog(this) == DialogResult.OK)
                    {
                        for (int i = 0; i < _keymap.Count; i++) _keymap[i].Key = copy[i].Key;
                    }
                }
            };

            var btnCancel = new Button
            {
                Text = L10n.T("Cancelar", "Cancel"),
                FlatStyle = FlatStyle.Flat,
                BackColor = UiTheme.Surface,
                ForeColor = UiTheme.Text,
                Location = new Point(214, 218),
                Size = new Size(88, 30),
                DialogResult = DialogResult.Cancel
            };
            btnCancel.FlatAppearance.BorderColor = UiTheme.Border;

            var btnSave = new Button
            {
                Text = L10n.T("Aplicar", "Apply"),
                FlatStyle = FlatStyle.Flat,
                BackColor = UiTheme.Cyan,
                ForeColor = Color.FromArgb(10, 12, 18),
                Font = new Font("Segoe UI Semibold", 9f, FontStyle.Bold),
                Location = new Point(310, 218),
                Size = new Size(90, 30)
            };
            btnSave.FlatAppearance.BorderSize = 0;
            btnSave.Click += (s, e) =>
            {
                MouseCamera = _cbCam.Checked;
                MouseInvertY = _cbInv.Checked;
                MouseSens = _tbSens.Value / 10f;
                MouseButtonsMode = _cmbBtns.SelectedIndex == 1 ? "lmb_shoot" : (_cmbBtns.SelectedIndex == 2 ? "off" : "rmb_shoot");
                DialogResult = DialogResult.OK;
                Close();
            };

            Controls.AddRange(new Control[] { lblHdr, _cbCam, _cbInv, lblSens, _tbSens, _lblSensVal, lblBtns, _cmbBtns, btnKeymap, btnCancel, btnSave });
            AcceptButton = btnSave;
            CancelButton = btnCancel;
            Shown += (s, e) => UiTheme.ApplyDarkFrame(Handle);
        }
    }

    sealed class LauncherForm : Form
    {
        private static readonly string DownloadsDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Downloads");
        private static readonly string RepoDir = Path.Combine(DownloadsDir, "victory-klbf-lab39");
        private static readonly string GLeafDir = Path.Combine(DownloadsDir, "victory-gleaf6");
        private static readonly string BackupsDir = Path.Combine(DownloadsDir, "victory-backups");
        private static readonly string DataDir = Path.Combine(DownloadsDir, "victory-data");
        private static readonly string ConfigPath = Path.Combine(DataDir, "launcher-config.ini");
        private static readonly string DiscordIdPath = Path.Combine(DataDir, "discord_app_id.txt");
        private static readonly string Motor1Ps1 = Path.Combine(RepoDir, "run-klbf-lab39.ps1");
        private static readonly string Motor2Ps1 = Path.Combine(RepoDir, "run-gleaf-ka1qux.ps1");
        private static readonly string GameDir = Path.Combine(DownloadsDir, "GTA6.Xbox.v1.0.2.1-CYBERLEEK");
        private static readonly string Motor1PrecompileBat = Path.Combine(GameDir, "PreCompilar-Shaders-KLBF-Lab39-AMD.bat");
        private static readonly string Motor2PrecompileBat = Path.Combine(GameDir, "PreCompilar-Shaders-GLeaf-Ka1qux-AMD.bat");

        private readonly List<BuildEntry> _m1Builds = new List<BuildEntry>();
        private readonly List<BuildEntry> _m2Builds = new List<BuildEntry>();
        private readonly List<PresetEntry> _m1Presets = new List<PresetEntry>();
        private readonly List<PresetEntry> _m2Presets = new List<PresetEntry>();
        private readonly List<KeyBindingItem> _keymap = new List<KeyBindingItem>();

        private int _engine = 1; // 1 = Motor 1 (KLBF Tiled), 2 = Motor 2 (GLeaf-6)
        private int _selectedBuildM1 = 0;
        private int _selectedBuildM2 = 0;
        private int _selectedPresetM1 = 0;
        private int _selectedPresetM2 = 0;

        private int _hoverBuild = -1;
        private int _hoverPreset = -1;
        // 0=Play, 1=Controls, 2=Shaders, 3=Logs, 4=DiscordCfg,
        // 5=LangPt, 6=LangEn, 7=Engine1, 8=Engine2,
        // 10=Unified, 11=Boost, 12=Lite, 13=Hud, 14=Discord, 15=Close
        private int _hoverCtrl = -1;

        private bool _enableHud = true;
        private bool _closeOnLaunch = true;
        private bool _unifiedMemory = true;
        private bool _liteGraphics = false;
        private bool _cpuGpuBoost = false;
        private bool _discordRpc = false;
        private bool _discordHideTitle = false;
        private string _discordAppId = "";
        private bool _mouseCamera = false;
        private bool _mouseInvertY = false;
        private float _mouseSens = 1.0f;
        private string _mouseButtons = "rmb_shoot";
        private bool _cacheM1Ok = false;
        private bool _cacheM2Ok = false;
        private string _status = "";
        private float _scale = 1f;

        private Rectangle _langPtRect, _langEnRect;
        private Rectangle _eng1Rect, _eng2Rect;
        private Rectangle[] _buildRects = new Rectangle[0];
        private Rectangle[] _presetRects = new Rectangle[0];
        private Rectangle _chkUnified, _chkBoost, _chkLite, _chkHud, _chkDiscord, _btnDiscordCfg, _chkClose;
        private Rectangle _btnControls, _btnShaders, _btnLogs, _btnPlay;

        [DllImport("kernel32.dll")]
        private static extern bool SetProcessWorkingSetSize(IntPtr proc, IntPtr min, IntPtr max);
        [DllImport("psapi.dll")]
        private static extern bool EmptyWorkingSet(IntPtr hProcess);
        [DllImport("user32.dll")]
        private static extern bool SetProcessDPIAware();

        private int S(int v) { return (int)Math.Round(v * _scale); }

        private List<BuildEntry> ActiveBuilds { get { return _engine == 1 ? _m1Builds : _m2Builds; } }
        private List<PresetEntry> ActivePresets { get { return _engine == 1 ? _m1Presets : _m2Presets; } }
        private int SelectedBuild
        {
            get { return _engine == 1 ? _selectedBuildM1 : _selectedBuildM2; }
            set { if (_engine == 1) _selectedBuildM1 = value; else _selectedBuildM2 = value; }
        }
        private int SelectedPreset
        {
            get { return _engine == 1 ? _selectedPresetM1 : _selectedPresetM2; }
            set { if (_engine == 1) _selectedPresetM1 = value; else _selectedPresetM2 = value; }
        }

        public LauncherForm()
        {
            Text = "GTA VI — VIctory";
            StartPosition = FormStartPosition.CenterScreen;
            FormBorderStyle = FormBorderStyle.FixedSingle;
            MaximizeBox = false;
            BackColor = UiTheme.Bg;
            ForeColor = UiTheme.Text;
            DoubleBuffered = true;
            KeyPreview = true;
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.OptimizedDoubleBuffer, true);

            using (var g = CreateGraphics()) { _scale = g.DpiX / 96f; }
            _cacheM1Ok = File.Exists(Path.Combine(DataDir, "cache-klbf-lab39", "precompiled.ok"));
            _cacheM2Ok = File.Exists(Path.Combine(DataDir, "cache-gleaf-ka1qux", "precompiled.ok"));

            InitDefaultKeymap();
            DiscoverBuilds();
            InitPresets();
            LoadConfig();

            ClientSize = new Size(S(576), S(602));
            ComputeLayout();

            MouseMove += OnMouseMoveCanvas;
            MouseLeave += (s, e) => { _hoverBuild = -1; _hoverPreset = -1; _hoverCtrl = -1; Cursor = Cursors.Default; Invalidate(); };
            MouseDown += OnMouseDownCanvas;
            MouseDoubleClick += OnMouseDoubleClickCanvas;
            KeyDown += OnKeyDownCanvas;
            Shown += (s, e) => { UiTheme.ApplyDarkFrame(Handle); TrimMemory(); };
        }

        private void DiscoverBuilds()
        {
            _m1Builds.Clear();
            string liveM1 = Path.Combine(RepoDir, "target", "release", "victory.exe");
            if (File.Exists(liveM1))
                _m1Builds.Add(new BuildEntry { Name = "Atual", Meta = "live", ExePath = liveM1, Available = true });

            if (Directory.Exists(BackupsDir))
            {
                string[] dirs = Directory.GetDirectories(BackupsDir, "motor1-v*");
                Array.Sort(dirs, StringComparer.OrdinalIgnoreCase);
                Array.Reverse(dirs);
                foreach (string dir in dirs)
                {
                    string exe = Path.Combine(dir, "victory.exe");
                    if (!File.Exists(exe)) continue;
                    string folder = Path.GetFileName(dir);
                    string ver = folder.StartsWith("motor1-", StringComparison.OrdinalIgnoreCase)
                        ? folder.Substring("motor1-".Length)
                        : folder;
                    string meta = ReadCommitMeta(Path.Combine(dir, "VERSION.txt"));
                    _m1Builds.Add(new BuildEntry { Name = ver, Meta = meta, ExePath = exe, Available = true });
                }
            }
            if (_m1Builds.Count == 0)
                _m1Builds.Add(new BuildEntry { Name = "N/A", Meta = "missing", ExePath = "", Available = false });

            _m2Builds.Clear();
            string liveM2 = Path.Combine(GLeafDir, "target", "release", "victory.exe");
            _m2Builds.Add(new BuildEntry
            {
                Name = "GLeaf-6",
                Meta = File.Exists(liveM2) ? "Ka1qux" : "missing",
                ExePath = liveM2,
                Available = File.Exists(liveM2) && File.Exists(Motor2Ps1)
            });
            if (Directory.Exists(BackupsDir))
            {
                string[] dirs2 = Directory.GetDirectories(BackupsDir, "motor2-v*");
                Array.Sort(dirs2, StringComparer.OrdinalIgnoreCase);
                Array.Reverse(dirs2);
                foreach (string dir in dirs2)
                {
                    string exe = Path.Combine(dir, "victory.exe");
                    if (!File.Exists(exe)) continue;
                    string folder = Path.GetFileName(dir);
                    string ver = folder.StartsWith("motor2-", StringComparison.OrdinalIgnoreCase)
                        ? folder.Substring("motor2-".Length)
                        : folder;
                    _m2Builds.Add(new BuildEntry { Name = ver, Meta = ReadCommitMeta(Path.Combine(dir, "VERSION.txt")), ExePath = exe, Available = true });
                }
            }
        }

        private static string ReadCommitMeta(string versionFile)
        {
            if (!File.Exists(versionFile)) return "";
            try
            {
                foreach (string ln in File.ReadAllLines(versionFile))
                    if (ln.StartsWith("commit=", StringComparison.OrdinalIgnoreCase))
                        return ln.Substring(7).Trim();
            }
            catch { }
            return "";
        }

        private void InitPresets()
        {
            _m1Presets.Clear();
            _m1Presets.Add(new PresetEntry { TitlePt = "FSR 2.4 + FrameGen", TitleEn = "FSR 2.4 + FrameGen", SubPt = "720p • Maior fluidez na RX 6600", SubEn = "720p • Highest smoothness on RX 6600", TagPt = "RECOMENDADO", TagEn = "RECOMMENDED", TagColor = UiTheme.Green, Args = "-FrameGen -Res720p" });
            _m1Presets.Add(new PresetEntry { TitlePt = "FSR 2.4", TitleEn = "FSR 2.4", SubPt = "720p • Sem geração de quadros", SubEn = "720p • No frame generation", TagPt = "720p", TagEn = "720p", TagColor = UiTheme.Cyan, Args = "-Res720p" });
            _m1Presets.Add(new PresetEntry { TitlePt = "Simple Ultraleve", TitleEn = "Simple Ultralight", SubPt = "720p • Menor consumo de RAM e VRAM", SubEn = "720p • Lowest RAM & VRAM usage", TagPt = "LEVE", TagEn = "LIGHT", TagColor = UiTheme.Blue, Args = "-SimpleUpscale -Res720p" });
            _m1Presets.Add(new PresetEntry { TitlePt = "FSR 2.4 Full HD", TitleEn = "FSR 2.4 Full HD", SubPt = "1080p • Resolução nativa", SubEn = "1080p • Native resolution", TagPt = "1080p", TagEn = "1080p", TagColor = UiTheme.Slate, Args = "" });
            _m1Presets.Add(new PresetEntry { TitlePt = "Modo Seguro", TitleEn = "Safe Mode", SubPt = "720p • Flags conservadoras", SubEn = "720p • Conservative flags", TagPt = "SAFE", TagEn = "SAFE", TagColor = UiTheme.Amber, Args = "-Safe -Res720p" });

            _m2Presets.Clear();
            _m2Presets.Add(new PresetEntry { TitlePt = "FSR + FrameGen (GLeaf-6)", TitleEn = "FSR + FrameGen (GLeaf-6)", SubPt = "720p • Geração de quadros AMD + Shaders Ka1qux", SubEn = "720p • AMD FrameGen + Ka1qux Shaders", TagPt = "M2 + FG", TagEn = "E2 + FG", TagColor = UiTheme.Pink, Args = "-FrameGen -Res720p" });
            _m2Presets.Add(new PresetEntry { TitlePt = "FSR 720p (GLeaf-6)", TitleEn = "FSR 720p (GLeaf-6)", SubPt = "720p • Sem FrameGen (estável)", SubEn = "720p • No FrameGen (stable)", TagPt = "M2 720p", TagEn = "E2 720p", TagColor = UiTheme.Blue, Args = "-Res720p" });
            _m2Presets.Add(new PresetEntry { TitlePt = "FSR 1080p (GLeaf-6)", TitleEn = "FSR 1080p (GLeaf-6)", SubPt = "1080p • Resolução nativa", SubEn = "1080p • Native resolution", TagPt = "M2 1080p", TagEn = "E2 1080p", TagColor = UiTheme.Slate, Args = "" });
        }

        private void InitDefaultKeymap()
        {
            _keymap.Clear();
            Action<string, string, string, string, Color> add = (id, pt, en, def, col) =>
                _keymap.Add(new KeyBindingItem { Id = id, LabelPt = pt, LabelEn = en, DefaultKey = def, Key = def, Accent = col });
            add("A", "Botão A", "Button A", "ENTER", UiTheme.Green);
            add("B", "Botão B", "Button B", "BACKSPACE", UiTheme.Red);
            add("X", "Botão X", "Button X", "X", UiTheme.Blue);
            add("Y", "Botão Y", "Button Y", "Y", UiTheme.Amber);
            add("LB", "LB / L1", "LB / L1", "Q", UiTheme.Cyan);
            add("RB", "RB / R1", "RB / R1", "E", UiTheme.Cyan);
            add("LT", "LT / L2", "LT / L2", "Z", UiTheme.Pink);
            add("RT", "RT / R2", "RT / R2", "C", UiTheme.Pink);
            add("MENU", "Start / Menu", "Start / Menu", "ESC", UiTheme.Slate);
            add("VIEW", "Back / View", "Back / View", "TAB", UiTheme.Slate);
            add("LS", "L3", "L3", "F", UiTheme.Slate);
            add("RS", "R3", "R3", "H", UiTheme.Slate);
            add("UP", "D-Pad Cima", "D-Pad Up", "UP", UiTheme.Cyan);
            add("DOWN", "D-Pad Baixo", "D-Pad Down", "DOWN", UiTheme.Cyan);
            add("LEFT", "D-Pad Esq", "D-Pad Left", "LEFT", UiTheme.Cyan);
            add("RIGHT", "D-Pad Dir", "D-Pad Right", "RIGHT", UiTheme.Cyan);
            add("LY+", "Frente (LS ↑)", "Forward (LS ↑)", "W", UiTheme.Green);
            add("LY-", "Trás (LS ↓)", "Back (LS ↓)", "S", UiTheme.Green);
            add("LX-", "Esq (LS ←)", "Left (LS ←)", "A", UiTheme.Green);
            add("LX+", "Dir (LS →)", "Right (LS →)", "D", UiTheme.Green);
            add("RY+", "Câmera ↑", "Camera ↑", "I", UiTheme.Amber);
            add("RY-", "Câmera ↓", "Camera ↓", "K", UiTheme.Amber);
            add("RX-", "Câmera ←", "Camera ←", "J", UiTheme.Amber);
            add("RX+", "Câmera →", "Camera →", "L", UiTheme.Amber);
        }

        private string SerializeCustomKeymap()
        {
            var diffs = new List<string>();
            foreach (var it in _keymap)
                if (!string.Equals(it.Key, it.DefaultKey, StringComparison.OrdinalIgnoreCase))
                    diffs.Add(it.Id + "=" + (string.IsNullOrEmpty(it.Key) ? "NONE" : it.Key));
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
                    if (string.Equals(it.Id, id, StringComparison.OrdinalIgnoreCase)) { it.Key = val; break; }
            }
        }

        private void LoadConfig()
        {
            try
            {
                if (File.Exists(DiscordIdPath))
                    _discordAppId = File.ReadAllText(DiscordIdPath).Trim();
            }
            catch { }

            try
            {
                if (!File.Exists(ConfigPath)) return;
                foreach (string raw in File.ReadAllLines(ConfigPath))
                {
                    int eq = raw.IndexOf('=');
                    if (eq <= 0) continue;
                    string k = raw.Substring(0, eq).Trim();
                    string v = raw.Substring(eq + 1).Trim();
                    if (k == "lang") L10n.IsEn = string.Equals(v, "en", StringComparison.OrdinalIgnoreCase);
                    else if (k == "engine") _engine = (v == "2") ? 2 : 1;
                    else if (k == "preset")
                    {
                        int idx;
                        if (int.TryParse(v, out idx) && idx >= 0 && idx < _m1Presets.Count) _selectedPresetM1 = idx;
                    }
                    else if (k == "preset_m2")
                    {
                        int idx;
                        if (int.TryParse(v, out idx) && idx >= 0 && idx < _m2Presets.Count) _selectedPresetM2 = idx;
                    }
                    else if (k == "build_version")
                    {
                        for (int i = 0; i < _m1Builds.Count; i++)
                            if (string.Equals(_m1Builds[i].Name, v, StringComparison.OrdinalIgnoreCase)) { _selectedBuildM1 = i; break; }
                    }
                    else if (k == "hud") _enableHud = (v == "1");
                    else if (k == "close_on_launch") _closeOnLaunch = (v == "1");
                    else if (k == "unified_memory") _unifiedMemory = (v != "0");
                    else if (k == "lite_graphics") _liteGraphics = (v == "1");
                    else if (k == "cpu_gpu_boost") _cpuGpuBoost = (v == "1");
                    else if (k == "discord_rpc") _discordRpc = (v == "1");
                    else if (k == "discord_hide_title") _discordHideTitle = (v == "1");
                    else if (k == "mouse_camera") _mouseCamera = (v == "1");
                    else if (k == "mouse_invert_y") _mouseInvertY = (v == "1");
                    else if (k == "mouse_sens")
                    {
                        float s;
                        if (float.TryParse(v, NumberStyles.Float, CultureInfo.InvariantCulture, out s))
                            _mouseSens = Math.Max(0.2f, Math.Min(4.0f, s));
                    }
                    else if (k == "mouse_buttons" && (v == "rmb_shoot" || v == "lmb_shoot" || v == "off")) _mouseButtons = v;
                    else if (k == "keymap") ApplyKeymapString(v);
                }
            }
            catch { }
        }

        private void SaveConfig()
        {
            try
            {
                Directory.CreateDirectory(DataDir);
                string buildName = (_selectedBuildM1 >= 0 && _selectedBuildM1 < _m1Builds.Count) ? _m1Builds[_selectedBuildM1].Name : "Atual";
                File.WriteAllLines(ConfigPath, new[]
                {
                    "lang=" + (L10n.IsEn ? "en" : "pt"),
                    "engine=" + _engine,
                    "preset=" + _selectedPresetM1,
                    "preset_m2=" + _selectedPresetM2,
                    "build_version=" + buildName,
                    "hud=" + (_enableHud ? "1" : "0"),
                    "close_on_launch=" + (_closeOnLaunch ? "1" : "0"),
                    "unified_memory=" + (_unifiedMemory ? "1" : "0"),
                    "lite_graphics=" + (_liteGraphics ? "1" : "0"),
                    "cpu_gpu_boost=" + (_cpuGpuBoost ? "1" : "0"),
                    "discord_rpc=" + (_discordRpc ? "1" : "0"),
                    "discord_hide_title=" + (_discordHideTitle ? "1" : "0"),
                    "mouse_camera=" + (_mouseCamera ? "1" : "0"),
                    "mouse_invert_y=" + (_mouseInvertY ? "1" : "0"),
                    "mouse_sens=" + _mouseSens.ToString("0.00", CultureInfo.InvariantCulture),
                    "mouse_buttons=" + _mouseButtons,
                    "keymap=" + SerializeCustomKeymap()
                });
                if (!string.IsNullOrEmpty(_discordAppId))
                    File.WriteAllText(DiscordIdPath, _discordAppId);
                else if (File.Exists(DiscordIdPath))
                    File.Delete(DiscordIdPath);
            }
            catch { }
        }

        private void ComputeLayout()
        {
            int pad = S(22);
            int w = ClientSize.Width - pad * 2;

            // Language flag switcher in top-right header
            int flagBtnW = S(52);
            int flagBtnH = S(22);
            _langEnRect = new Rectangle(ClientSize.Width - pad - flagBtnW, S(16), flagBtnW, flagBtnH);
            _langPtRect = new Rectangle(_langEnRect.X - S(6) - flagBtnW, S(16), flagBtnW, flagBtnH);

            // Engine 1 / Engine 2 segmented switcher
            int engY = S(72);
            int engGap = S(8);
            int engW = (w - engGap) / 2;
            _eng1Rect = new Rectangle(pad, engY, engW, S(36));
            _eng2Rect = new Rectangle(pad + engW + engGap, engY, engW, S(36));

            // Version selector row for active engine
            var builds = ActiveBuilds;
            int visibleBuilds = Math.Min(builds.Count, 5);
            _buildRects = new Rectangle[visibleBuilds];
            int bGap = S(6);
            int bW = (w - bGap * (visibleBuilds - 1)) / Math.Max(1, visibleBuilds);
            int bY = engY + S(54);
            for (int i = 0; i < visibleBuilds; i++)
                _buildRects[i] = new Rectangle(pad + i * (bW + bGap), bY, bW, S(32));

            // Presets area (fixed height for 5 slots so footer stays rock-steady)
            var presets = ActivePresets;
            _presetRects = new Rectangle[presets.Count];
            int pY = bY + S(52);
            for (int i = 0; i < presets.Count; i++)
            {
                _presetRects[i] = new Rectangle(pad, pY + i * S(46), w, S(41));
            }

            // 2-column compact options grid (aligned below 5 preset slots)
            int optY = pY + 5 * S(46) + S(8);
            int colW = (w - S(12)) / 2;
            int col2X = pad + colW + S(12);
            _chkUnified = new Rectangle(pad, optY, colW, S(24));
            _chkBoost = new Rectangle(col2X, optY, colW, S(24));

            _chkLite = new Rectangle(pad, optY + S(27), colW, S(24));
            _chkHud = new Rectangle(col2X, optY + S(27), colW, S(24));

            _chkDiscord = new Rectangle(pad, optY + S(54), colW - S(62), S(24));
            _btnDiscordCfg = new Rectangle(pad + colW - S(58), optY + S(55), S(56), S(22));
            _chkClose = new Rectangle(col2X, optY + S(54), colW, S(24));

            // Bottom action bar
            int btnY = optY + S(88);
            _btnControls = new Rectangle(pad, btnY, S(100), S(38));
            _btnShaders = new Rectangle(pad + S(106), btnY, S(82), S(38));
            _btnLogs = new Rectangle(pad + S(194), btnY, S(68), S(38));
            _btnPlay = new Rectangle(ClientSize.Width - pad - S(254), btnY, S(254), S(38));
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            Graphics g = e.Graphics;
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

            Color engAccent = (_engine == 1) ? UiTheme.Cyan : UiTheme.Pink;

            // Top 2px gradient accent
            using (var grad = new LinearGradientBrush(new Rectangle(0, 0, ClientSize.Width, S(2)), UiTheme.Pink, UiTheme.Cyan, 0f))
                g.FillRectangle(grad, 0, 0, ClientSize.Width, S(2));

            int pad = S(22);
            using (var fTitle = new Font("Segoe UI Semibold", 15f, FontStyle.Bold))
            using (var fSub = new Font("Segoe UI", 8.1f))
            using (var fSec = new Font("Segoe UI Semibold", 7.1f, FontStyle.Bold))
            using (var fItem = new Font("Segoe UI Semibold", 9.0f, FontStyle.Bold))
            using (var fItemSub = new Font("Segoe UI", 7.8f))
            using (var fPill = new Font("Segoe UI Semibold", 7.2f, FontStyle.Bold))
            using (var fMicro = new Font("Segoe UI", 6.8f))
            using (var bTxt = new SolidBrush(UiTheme.Text))
            using (var bMut = new SolidBrush(UiTheme.Muted))
            using (var sfC = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center })
            {
                // Header title + engine badge
                g.DrawString("GTA VI", fTitle, bTxt, pad, S(13));
                float wGta = g.MeasureString("GTA VI", fTitle).Width;
                using (var bAcc = new SolidBrush(engAccent))
                    g.DrawString("VICTORY", fTitle, bAcc, pad + wGta - S(2), S(13));

                string activeEngTag = _engine == 1 ? L10n.T("MOTOR 1", "ENGINE 1") : L10n.T("MOTOR 2", "ENGINE 2");
                bool cacheOk = _engine == 1 ? _cacheM1Ok : _cacheM2Ok;
                string cacheStr = cacheOk ? L10n.T("Cache OK", "Cache OK") : L10n.T("Cache Frio", "Cold Cache");
                g.DrawString(activeEngTag + "  •  RX 6600 8 GB  •  " + cacheStr, fSub, bMut, pad, S(41));

                // Language switcher pills with vector flags
                DrawLangButton(g, _langPtRect, true, !L10n.IsEn, _hoverCtrl == 5, fPill, sfC);
                DrawLangButton(g, _langEnRect, false, L10n.IsEn, _hoverCtrl == 6, fPill, sfC);

                // Section 0: Engine Selector (Motor 1 vs Motor 2)
                g.DrawString(L10n.T("SELECIONAR MOTOR", "SELECT ENGINE"), fSec, bMut, pad, S(58));
                DrawEngineCard(
                    g, _eng1Rect,
                    L10n.T("MOTOR 1", "ENGINE 1"),
                    "KLBF Lab 39 • Tiled Pool",
                    UiTheme.Cyan, _engine == 1, _hoverCtrl == 7, fPill, fMicro);
                DrawEngineCard(
                    g, _eng2Rect,
                    L10n.T("MOTOR 2", "ENGINE 2"),
                    "GLeaf-6 • Shaders Ka1qux",
                    UiTheme.Pink, _engine == 2, _hoverCtrl == 8, fPill, fMicro);

                // Section 1: Version selector for active engine
                string verHeader = _engine == 1
                    ? L10n.T("VERSÃO DO MOTOR 1", "ENGINE 1 VERSION")
                    : L10n.T("VERSÃO DO MOTOR 2", "ENGINE 2 VERSION");
                g.DrawString(verHeader, fSec, bMut, pad, S(113));

                var builds = ActiveBuilds;
                for (int i = 0; i < _buildRects.Length; i++)
                {
                    BuildEntry b = builds[i];
                    Rectangle r = _buildRects[i];
                    bool sel = (i == SelectedBuild);
                    bool hov = (i == _hoverBuild);
                    Color bg = sel ? UiTheme.SurfaceActive : (hov ? UiTheme.SurfaceHover : UiTheme.Surface);
                    Color bd = sel ? engAccent : (hov ? UiTheme.BorderHover : UiTheme.Border);

                    using (var path = UiTheme.RoundedRect(r, S(7)))
                    using (var bBg = new SolidBrush(bg))
                    using (var pen = new Pen(bd, sel ? 1.4f : 1f))
                    {
                        g.FillPath(bBg, path);
                        g.DrawPath(pen, path);
                    }
                    string bLabel = (b.Name == "Atual" && L10n.IsEn) ? "Latest" : b.Name;
                    using (var bName = new SolidBrush(sel ? engAccent : (b.Available ? UiTheme.Text : UiTheme.Dim)))
                    {
                        Rectangle nr = string.IsNullOrEmpty(b.Meta) ? r : new Rectangle(r.X, r.Y + S(2), r.Width, S(16));
                        g.DrawString(bLabel, fPill, bName, nr, sfC);
                    }
                    if (!string.IsNullOrEmpty(b.Meta))
                    {
                        Rectangle mr = new Rectangle(r.X, r.Y + S(17), r.Width, S(12));
                        g.DrawString(b.Meta, fMicro, bMut, mr, sfC);
                    }
                }

                // Section 2: Presets
                g.DrawString(L10n.T("PERFIL GRÁFICO", "GRAPHICS PRESET"), fSec, bMut, pad, S(164));
                var presets = ActivePresets;
                for (int i = 0; i < _presetRects.Length; i++)
                {
                    PresetEntry pItem = presets[i];
                    Rectangle r = _presetRects[i];
                    bool sel = (i == SelectedPreset);
                    bool hov = (i == _hoverPreset);

                    Color bg = sel ? UiTheme.SurfaceActive : (hov ? UiTheme.SurfaceHover : UiTheme.Surface);
                    Color bd = sel ? engAccent : (hov ? UiTheme.BorderHover : UiTheme.Border);
                    using (var path = UiTheme.RoundedRect(r, S(8)))
                    using (var bBg = new SolidBrush(bg))
                    using (var pen = new Pen(bd, sel ? 1.4f : 1f))
                    {
                        g.FillPath(bBg, path);
                        g.DrawPath(pen, path);
                    }
                    if (sel)
                    {
                        Rectangle bar = new Rectangle(r.X + 1, r.Y + S(8), S(3), r.Height - S(16));
                        using (var grad = new LinearGradientBrush(bar, UiTheme.Pink, UiTheme.Cyan, 90f))
                            g.FillRectangle(grad, bar);
                    }

                    g.DrawString(L10n.IsEn ? pItem.TitleEn : pItem.TitlePt, fItem, bTxt, r.X + S(13), r.Y + S(4));
                    g.DrawString(L10n.IsEn ? pItem.SubEn : pItem.SubPt, fItemSub, bMut, r.X + S(13), r.Y + S(22));

                    string tag = L10n.IsEn ? pItem.TagEn : pItem.TagPt;
                    int tw = (int)g.MeasureString(tag, fPill).Width + S(14);
                    Rectangle tr = new Rectangle(r.Right - tw - S(10), r.Y + S(10), tw, S(20));
                    using (var tp = UiTheme.RoundedRect(tr, S(9)))
                    using (var tb = new SolidBrush(Color.FromArgb(26, pItem.TagColor)))
                    using (var tt = new SolidBrush(pItem.TagColor))
                    {
                        g.FillPath(tb, tp);
                        g.DrawString(tag, fPill, tt, tr, sfC);
                    }
                }

                // Section 3: Compact 2-column toggles
                bool isM1 = (_engine == 1);
                DrawCheck(g, _chkUnified, _unifiedMemory, L10n.T("Memória Unificada (Tiled)", "Unified Memory (Tiled)"), UiTheme.Cyan, _hoverCtrl == 10, isM1);
                DrawCheck(g, _chkBoost, _cpuGpuBoost, L10n.T("Boost CPU↔GPU (v1.2)", "CPU↔GPU Boost (v1.2)"), UiTheme.Green, _hoverCtrl == 11, isM1);
                DrawCheck(g, _chkLite, _liteGraphics, isM1 ? L10n.T("Modo Gráfico Leve", "Lite Graphics Mode") : L10n.T("Baixa Memória (512px)", "Low Memory (512px)"), UiTheme.Amber, _hoverCtrl == 12, true);
                DrawCheck(g, _chkHud, _enableHud, L10n.T("HUD de FPS", "FPS Overlay HUD"), UiTheme.Cyan, _hoverCtrl == 13, true);

                string rpcLbl = _discordRpc && string.IsNullOrEmpty(_discordAppId)
                    ? L10n.T("Discord RPC (sem ID)", "Discord RPC (no ID)")
                    : "Discord Rich Presence";
                DrawCheck(g, _chkDiscord, _discordRpc, rpcLbl, UiTheme.Blurple, _hoverCtrl == 14, isM1);
                DrawMiniPill(g, _btnDiscordCfg, string.IsNullOrEmpty(_discordAppId) ? "Set ID" : "ID ✓", UiTheme.Blurple, _hoverCtrl == 4, isM1, sfC);
                DrawCheck(g, _chkClose, _closeOnLaunch, L10n.T("Fechar ao Jogar", "Close on Launch"), UiTheme.Slate, _hoverCtrl == 15, true);

                // Section 4: Bottom Action Bar
                DrawActionBtn(g, _btnControls, _mouseCamera ? L10n.T("Controles •", "Controls •") : L10n.T("Controles", "Controls"), false, _hoverCtrl == 1, sfC);
                DrawActionBtn(g, _btnShaders, "Shaders", false, _hoverCtrl == 2, sfC);
                DrawActionBtn(g, _btnLogs, "Logs", false, _hoverCtrl == 3, sfC);

                string verTag = (SelectedBuild >= 0 && SelectedBuild < builds.Count) ? builds[SelectedBuild].Name : "";
                if (verTag == "Atual" && L10n.IsEn) verTag = "Latest";
                string shortEng = _engine == 1 ? L10n.T("M1", "E1") : L10n.T("M2", "E2");
                string playText = L10n.T("▶  JOGAR · ", "▶  PLAY · ") + shortEng + " (" + verTag + ")";
                DrawActionBtn(g, _btnPlay, playText, true, _hoverCtrl == 0, sfC);

                if (!string.IsNullOrEmpty(_status))
                {
                    using (var bSt = new SolidBrush(UiTheme.Amber))
                        g.DrawString(_status, fSub, bSt, pad, ClientSize.Height - S(20));
                }
            }
        }

        private void DrawLangButton(Graphics g, Rectangle r, bool isBrazil, bool active, bool hov, Font fPill, StringFormat sf)
        {
            Color bg = active ? UiTheme.SurfaceActive : (hov ? UiTheme.SurfaceHover : UiTheme.Surface);
            Color bd = active ? UiTheme.Cyan : (hov ? UiTheme.BorderHover : UiTheme.Border);
            using (var p = UiTheme.RoundedRect(r, S(6)))
            using (var bf = new SolidBrush(bg))
            using (var pen = new Pen(bd, active ? 1.3f : 1f))
            {
                g.FillPath(bf, p);
                g.DrawPath(pen, p);
            }
            Rectangle flagR = new Rectangle(r.X + S(6), r.Y + S(5), S(18), S(12));
            if (isBrazil) UiTheme.DrawBrazilFlag(g, flagR);
            else UiTheme.DrawUsFlag(g, flagR);

            Rectangle txtR = new Rectangle(flagR.Right + S(3), r.Y, r.Right - flagR.Right - S(5), r.Height);
            using (var bt = new SolidBrush(active ? UiTheme.Text : UiTheme.Muted))
                g.DrawString(isBrazil ? "PT" : "EN", fPill, bt, txtR, sf);
        }

        private void DrawEngineCard(Graphics g, Rectangle r, string name, string desc, Color accent, bool active, bool hov, Font fName, Font fDesc)
        {
            Color bg = active ? Color.FromArgb(26, accent) : (hov ? UiTheme.SurfaceHover : UiTheme.Surface);
            Color bd = active ? accent : (hov ? UiTheme.BorderHover : UiTheme.Border);
            using (var p = UiTheme.RoundedRect(r, S(8)))
            using (var bf = new SolidBrush(bg))
            using (var pen = new Pen(bd, active ? 1.5f : 1f))
            {
                g.FillPath(bf, p);
                g.DrawPath(pen, p);
            }
            using (var bDot = new SolidBrush(active ? accent : UiTheme.Dim))
                g.FillEllipse(bDot, r.X + S(11), r.Y + S(14), S(7), S(7));
            using (var bt = new SolidBrush(active ? accent : UiTheme.Text))
                g.DrawString(name, fName, bt, r.X + S(24), r.Y + S(5));
            using (var bm = new SolidBrush(active ? UiTheme.Text : UiTheme.Muted))
                g.DrawString(desc, fDesc, bm, r.X + S(24), r.Y + S(19));
        }

        private void DrawCheck(Graphics g, Rectangle r, bool on, string label, Color accent, bool hov, bool enabled)
        {
            Rectangle box = new Rectangle(r.X, r.Y + S(4), S(14), S(14));
            using (var bp = UiTheme.RoundedRect(box, S(4)))
            using (var bf = new SolidBrush(!enabled ? UiTheme.SurfaceDisabled : (on ? accent : UiTheme.Surface)))
            using (var pen = new Pen(!enabled ? UiTheme.Border : (on ? accent : (hov ? UiTheme.BorderHover : UiTheme.Border)), 1.2f))
            {
                g.FillPath(bf, bp);
                g.DrawPath(pen, bp);
            }
            if (on && enabled)
            {
                using (var p = new Pen(UiTheme.Bg, 1.9f))
                {
                    g.DrawLine(p, box.X + S(3), box.Y + S(7), box.X + S(6), box.Y + S(10));
                    g.DrawLine(p, box.X + S(6), box.Y + S(10), box.X + S(11), box.Y + S(4));
                }
            }
            Color tc = !enabled ? UiTheme.Dim : ((on || hov) ? UiTheme.Text : UiTheme.Muted);
            using (var f = new Font("Segoe UI", 8.3f))
            using (var b = new SolidBrush(tc))
                g.DrawString(label, f, b, box.Right + S(7), r.Y + S(3));
        }

        private void DrawMiniPill(Graphics g, Rectangle r, string txt, Color col, bool hov, bool enabled, StringFormat sf)
        {
            Color c = enabled ? col : UiTheme.Dim;
            using (var p = UiTheme.RoundedRect(r, S(5)))
            using (var bf = new SolidBrush(!enabled ? UiTheme.SurfaceDisabled : (hov ? Color.FromArgb(46, c) : Color.FromArgb(24, c))))
            using (var pen = new Pen(Color.FromArgb(!enabled ? 50 : (hov ? 180 : 100), c), 1f))
            using (var f = new Font("Segoe UI Semibold", 7.2f, FontStyle.Bold))
            using (var bt = new SolidBrush(c))
            {
                g.FillPath(bf, p);
                g.DrawPath(pen, p);
                g.DrawString(txt, f, bt, r, sf);
            }
        }

        private void DrawActionBtn(Graphics g, Rectangle r, string text, bool primary, bool hov, StringFormat sf)
        {
            using (var path = UiTheme.RoundedRect(r, S(8)))
            {
                if (primary)
                {
                    Color c1 = hov ? Color.FromArgb(255, 86, 164) : UiTheme.Pink;
                    Color c2 = hov ? Color.FromArgb(62, 228, 250) : UiTheme.Cyan;
                    using (var grad = new LinearGradientBrush(r, c1, c2, 0f))
                        g.FillPath(grad, path);
                }
                else
                {
                    using (var bf = new SolidBrush(hov ? UiTheme.SurfaceHover : UiTheme.Surface))
                    using (var pen = new Pen(hov ? UiTheme.BorderHover : UiTheme.Border, 1f))
                    {
                        g.FillPath(bf, path);
                        g.DrawPath(pen, path);
                    }
                }
            }
            using (var f = new Font("Segoe UI Semibold", primary ? 9.1f : 8.3f, FontStyle.Bold))
            using (var bt = new SolidBrush(primary ? Color.FromArgb(10, 12, 18) : UiTheme.Text))
                g.DrawString(text, f, bt, r, sf);
        }

        private void OnMouseMoveCanvas(object sender, MouseEventArgs e)
        {
            int ob = _hoverBuild, op = _hoverPreset, oc = _hoverCtrl;
            _hoverBuild = -1; _hoverPreset = -1; _hoverCtrl = -1;

            for (int i = 0; i < _buildRects.Length; i++)
                if (_buildRects[i].Contains(e.Location)) { _hoverBuild = i; break; }
            for (int i = 0; i < _presetRects.Length; i++)
                if (_presetRects[i].Contains(e.Location)) { _hoverPreset = i; break; }

            if (_btnPlay.Contains(e.Location)) _hoverCtrl = 0;
            else if (_btnControls.Contains(e.Location)) _hoverCtrl = 1;
            else if (_btnShaders.Contains(e.Location)) _hoverCtrl = 2;
            else if (_btnLogs.Contains(e.Location)) _hoverCtrl = 3;
            else if (_btnDiscordCfg.Contains(e.Location) && _engine == 1) _hoverCtrl = 4;
            else if (_langPtRect.Contains(e.Location)) _hoverCtrl = 5;
            else if (_langEnRect.Contains(e.Location)) _hoverCtrl = 6;
            else if (_eng1Rect.Contains(e.Location)) _hoverCtrl = 7;
            else if (_eng2Rect.Contains(e.Location)) _hoverCtrl = 8;
            else if (_chkUnified.Contains(e.Location) && _engine == 1) _hoverCtrl = 10;
            else if (_chkBoost.Contains(e.Location) && _engine == 1) _hoverCtrl = 11;
            else if (_chkLite.Contains(e.Location)) _hoverCtrl = 12;
            else if (_chkHud.Contains(e.Location)) _hoverCtrl = 13;
            else if (_chkDiscord.Contains(e.Location) && _engine == 1) _hoverCtrl = 14;
            else if (_chkClose.Contains(e.Location)) _hoverCtrl = 15;

            Cursor = (_hoverBuild >= 0 || _hoverPreset >= 0 || _hoverCtrl >= 0) ? Cursors.Hand : Cursors.Default;
            if (ob != _hoverBuild || op != _hoverPreset || oc != _hoverCtrl) Invalidate();
        }

        private void OnMouseDownCanvas(object sender, MouseEventArgs e)
        {
            if (e.Button != MouseButtons.Left) return;

            if (_langPtRect.Contains(e.Location))
            {
                L10n.IsEn = false; _status = ""; SaveConfig(); Invalidate(); return;
            }
            if (_langEnRect.Contains(e.Location))
            {
                L10n.IsEn = true; _status = ""; SaveConfig(); Invalidate(); return;
            }
            if (_eng1Rect.Contains(e.Location))
            {
                if (_engine != 1) { _engine = 1; _status = ""; ComputeLayout(); SaveConfig(); Invalidate(); }
                return;
            }
            if (_eng2Rect.Contains(e.Location))
            {
                if (_engine != 2) { _engine = 2; _status = ""; ComputeLayout(); SaveConfig(); Invalidate(); }
                return;
            }

            for (int i = 0; i < _buildRects.Length; i++)
            {
                if (_buildRects[i].Contains(e.Location))
                {
                    SelectedBuild = i;
                    _status = L10n.T("Versão selecionada: ", "Selected version: ") + ActiveBuilds[i].Name;
                    SaveConfig(); Invalidate(); return;
                }
            }
            for (int i = 0; i < _presetRects.Length; i++)
            {
                if (_presetRects[i].Contains(e.Location))
                {
                    SelectedPreset = i; _status = ""; SaveConfig(); Invalidate(); return;
                }
            }

            if (_chkUnified.Contains(e.Location) && _engine == 1) { _unifiedMemory = !_unifiedMemory; SaveConfig(); Invalidate(); }
            else if (_chkBoost.Contains(e.Location) && _engine == 1) { _cpuGpuBoost = !_cpuGpuBoost; SaveConfig(); Invalidate(); }
            else if (_chkLite.Contains(e.Location)) { _liteGraphics = !_liteGraphics; SaveConfig(); Invalidate(); }
            else if (_chkHud.Contains(e.Location)) { _enableHud = !_enableHud; SaveConfig(); Invalidate(); }
            else if (_chkClose.Contains(e.Location)) { _closeOnLaunch = !_closeOnLaunch; SaveConfig(); Invalidate(); }
            else if (_chkDiscord.Contains(e.Location) && _engine == 1)
            {
                _discordRpc = !_discordRpc;
                if (_discordRpc && string.IsNullOrEmpty(_discordAppId)) OpenDiscordConfig();
                else { SaveConfig(); Invalidate(); }
            }
            else if (_btnDiscordCfg.Contains(e.Location) && _engine == 1) { OpenDiscordConfig(); }
            else if (_btnControls.Contains(e.Location)) { OpenControlsDialog(); }
            else if (_btnShaders.Contains(e.Location))
            {
                string bat = (_engine == 1) ? Motor1PrecompileBat : Motor2PrecompileBat;
                if (File.Exists(bat))
                    Process.Start(new ProcessStartInfo { FileName = bat, WorkingDirectory = Path.GetDirectoryName(bat), UseShellExecute = true });
                else { _status = L10n.T("Script de pré-compilação não encontrado.", "Precompile script not found."); Invalidate(); }
            }
            else if (_btnLogs.Contains(e.Location))
            {
                try { Process.Start("explorer.exe", DataDir); } catch { }
            }
            else if (_btnPlay.Contains(e.Location)) { LaunchSelected(); }
        }

        private void OnMouseDoubleClickCanvas(object sender, MouseEventArgs e)
        {
            if (e.Button != MouseButtons.Left) return;
            for (int i = 0; i < _presetRects.Length; i++)
                if (_presetRects[i].Contains(e.Location)) { SelectedPreset = i; SaveConfig(); LaunchSelected(); return; }
        }

        private void OnKeyDownCanvas(object sender, KeyEventArgs e)
        {
            var presets = ActivePresets;
            if (e.KeyCode == Keys.Up && SelectedPreset > 0) { SelectedPreset--; SaveConfig(); Invalidate(); }
            else if (e.KeyCode == Keys.Down && SelectedPreset < presets.Count - 1) { SelectedPreset++; SaveConfig(); Invalidate(); }
            else if (e.KeyCode == Keys.Left && SelectedBuild > 0) { SelectedBuild--; SaveConfig(); Invalidate(); }
            else if (e.KeyCode == Keys.Right && SelectedBuild < _buildRects.Length - 1) { SelectedBuild++; SaveConfig(); Invalidate(); }
            else if (e.KeyCode == Keys.Enter) LaunchSelected();
            else if (e.KeyCode == Keys.Escape) Close();
        }

        private void OpenDiscordConfig()
        {
            using (var dlg = new DiscordConfigForm(_discordAppId, _discordHideTitle))
            {
                if (dlg.ShowDialog(this) == DialogResult.OK)
                {
                    _discordAppId = dlg.AppIdValue;
                    _discordHideTitle = dlg.HideTitleValue;
                    if (!string.IsNullOrEmpty(_discordAppId)) _discordRpc = true;
                    _status = string.IsNullOrEmpty(_discordAppId)
                        ? L10n.T("Discord App ID limpo.", "Discord App ID cleared.")
                        : L10n.T("Discord RPC salvo (ID " + _discordAppId + ").", "Discord RPC saved (ID " + _discordAppId + ").");
                    SaveConfig();
                    Invalidate();
                }
            }
        }

        private void OpenControlsDialog()
        {
            using (var dlg = new ControlsForm(_mouseCamera, _mouseInvertY, _mouseSens, _mouseButtons, _keymap, _scale))
            {
                if (dlg.ShowDialog(this) == DialogResult.OK)
                {
                    _mouseCamera = dlg.MouseCamera;
                    _mouseInvertY = dlg.MouseInvertY;
                    _mouseSens = dlg.MouseSens;
                    _mouseButtons = dlg.MouseButtonsMode;
                    _status = L10n.T("Controles salvos.", "Controls saved.");
                    SaveConfig();
                    Invalidate();
                }
            }
        }

        private void LaunchSelected()
        {
            var builds = ActiveBuilds;
            var presets = ActivePresets;
            if (SelectedBuild < 0 || SelectedBuild >= builds.Count) return;
            BuildEntry build = builds[SelectedBuild];
            if (!build.Available || string.IsNullOrEmpty(build.ExePath) || !File.Exists(build.ExePath))
            {
                _status = L10n.T("Build não disponível em disco.", "Build not available on disk.");
                Invalidate();
                return;
            }

            PresetEntry preset = presets[SelectedPreset];
            SaveConfig();

            string script = (_engine == 1) ? Motor1Ps1 : Motor2Ps1;
            if (!File.Exists(script))
            {
                _status = L10n.T("Script não encontrado: ", "Script not found: ") + Path.GetFileName(script);
                Invalidate();
                return;
            }

            var parts = new List<string>();
            if (!string.IsNullOrEmpty(preset.Args)) parts.Add(preset.Args);
            if (_engine == 1)
            {
                parts.Add("-ExePath \"" + build.ExePath + "\"");
                if (!_enableHud) parts.Add("-NoHud");
                if (!_unifiedMemory) parts.Add("-NoUnifiedMemory");
                if (_liteGraphics) parts.Add("-LiteGraphics");
                if (_cpuGpuBoost) parts.Add("-CpuGpuBoost");
                if (_discordRpc)
                {
                    parts.Add("-DiscordRpc");
                    if (!string.IsNullOrEmpty(_discordAppId)) parts.Add("-DiscordAppId \"" + _discordAppId + "\"");
                    if (_discordHideTitle) parts.Add("-DiscordHideTitle");
                }
                if (_mouseCamera)
                {
                    parts.Add("-MouseCamera");
                    parts.Add("-MouseSens " + _mouseSens.ToString("0.00", CultureInfo.InvariantCulture));
                    if (_mouseInvertY) parts.Add("-MouseInvertY");
                }
                if (_mouseButtons != "off") parts.Add("-MouseButtons " + _mouseButtons);
                string km = SerializeCustomKeymap();
                if (!string.IsNullOrEmpty(km)) parts.Add("-Keymap \"" + km + "\"");
            }
            else
            {
                if (!_enableHud) parts.Add("-NoHud");
                if (_liteGraphics) parts.Add("-LowMemory");
            }

            try
            {
                Process.Start(new ProcessStartInfo
                {
                    FileName = "powershell.exe",
                    Arguments = "-NoProfile -ExecutionPolicy Bypass -File \"" + script + "\" " + string.Join(" ", parts.ToArray()),
                    WorkingDirectory = RepoDir,
                    UseShellExecute = true
                });

                if (_closeOnLaunch) Close();
                else
                {
                    string engLbl = _engine == 1 ? L10n.T("Motor 1", "Engine 1") : L10n.T("Motor 2", "Engine 2");
                    _status = L10n.T("Iniciado: ", "Launched: ") + engLbl + " • " + build.Name;
                    TrimMemory();
                    Invalidate();
                }
            }
            catch (Exception ex)
            {
                _status = L10n.T("Erro: ", "Error: ") + ex.Message;
                Invalidate();
            }
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
