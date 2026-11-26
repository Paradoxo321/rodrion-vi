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
        public string Name;      // e.g. "Atual", "v1.2.0", "v1.1.0-rc1", "v1.0.0"
        public string Meta;      // commit or date hint
        public string ExePath;
        public bool IsLatest;
    }

    sealed class PresetEntry
    {
        public string Title;
        public string Subtitle;
        public string Tag;
        public Color TagColor;
        public string Args;
    }

    sealed class KeyBindingItem
    {
        public string Id;
        public string Label;
        public string DefaultKey;
        public string Key;
        public Color Accent;
    }

    static class UiTheme
    {
        public static readonly Color Bg = Color.FromArgb(12, 14, 19);
        public static readonly Color Surface = Color.FromArgb(19, 22, 30);
        public static readonly Color SurfaceHover = Color.FromArgb(26, 31, 42);
        public static readonly Color SurfaceActive = Color.FromArgb(26, 35, 50);
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
                Text = "Cole o Application ID do Discord Developer Portal (somente dígitos):",
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
                Text = "Ocultar nome do jogo no status (mostrar apenas FPS e tempo)",
                Checked = hideTitle,
                ForeColor = UiTheme.Text,
                BackColor = UiTheme.Bg,
                Location = new Point(20, 106),
                Size = new Size(380, 24)
            };

            var lblPortal = new LinkLabel
            {
                Text = "Abrir Discord Developer Portal",
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
                Text = "Cancelar",
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
                Text = "Salvar",
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
                            MessageBox.Show(this, "O Application ID deve conter apenas números.", "Discord RPC", MessageBoxButtons.OK, MessageBoxIcon.Warning);
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
        private string _hint = "Clique em um botão do Xbox e pressione a tecla desejada (ESC cancela • DEL desativa).";

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
                g.DrawString("MAPEAMENTO XBOX → TECLADO", fTitle, bPri, pad, S(14));
                g.DrawString("Clique em qualquer ação para redefinir a tecla", fSub, bMut, pad, S(36));

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
                    {
                        g.FillEllipse(bDot, r.X + S(8), r.Y + S(8), S(7), S(7));
                    }
                    g.DrawString(item.Label, fRow, bPri, r.X + S(21), r.Y + S(4));

                    string keyText = listening ? "TECLA..." : (string.IsNullOrEmpty(item.Key) || item.Key == "NONE" ? "OFF" : item.Key);
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
                {
                    g.DrawString(_hint, fSub, bHint, pad, hintY);
                }

                DrawBtn(g, _resetRect, "Restaurar Padrão", false, _hoverBtn == 1, sfC);
                DrawBtn(g, _cancelRect, "Cancelar", false, _hoverBtn == 2, sfC);
                DrawBtn(g, _saveRect, "Salvar", true, _hoverBtn == 0, sfC);
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
            {
                g.DrawString(text, f, b, r, sf);
            }
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
                    _hint = _listeningIndex >= 0
                        ? "Pressione uma tecla para \"" + _items[i].Label + "\" (ESC cancela • DEL desativa)"
                        : "Clique em um botão do Xbox e pressione a tecla desejada.";
                    Invalidate();
                    return;
                }
            }
            if (_resetRect.Contains(e.Location))
            {
                _listeningIndex = -1;
                foreach (var it in _items) it.Key = it.DefaultKey;
                _hint = "Padrões restaurados.";
                Invalidate();
            }
            else if (_cancelRect.Contains(e.Location)) { DialogResult = DialogResult.Cancel; Close(); }
            else if (_saveRect.Contains(e.Location)) { DialogResult = DialogResult.OK; Close(); }
        }

        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            if (_listeningIndex >= 0)
            {
                HandleCapturedKey(keyData & Keys.KeyCode);
                return true;
            }
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
            if (code == Keys.Escape) { _listeningIndex = -1; _hint = "Cancelado."; Invalidate(); return; }
            string mapped = code == Keys.Delete ? "NONE" : KeyToSpec(code);
            if (string.IsNullOrEmpty(mapped)) { _hint = "Tecla não suportada."; Invalidate(); return; }
            var target = _items[_listeningIndex];
            if (mapped != "NONE")
                foreach (var other in _items)
                    if (other != target && string.Equals(other.Key, mapped, StringComparison.OrdinalIgnoreCase))
                        other.Key = "NONE";
            target.Key = mapped;
            _listeningIndex = -1;
            _hint = target.Label + " = " + mapped;
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

            Text = "Controles — Mouse & Teclado";
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
                Text = "MOUSE E MAPEAMENTO XBOX → TECLADO",
                Font = new Font("Segoe UI Semibold", 9.2f, FontStyle.Bold),
                ForeColor = UiTheme.Cyan,
                Location = new Point(20, 16),
                AutoSize = true
            };

            _cbCam = new CheckBox
            {
                Text = "Mouse como Câmera (Analógico Direito • F9 trava/solta)",
                Checked = cam,
                ForeColor = UiTheme.Text,
                Location = new Point(20, 46),
                Size = new Size(380, 24)
            };

            _cbInv = new CheckBox
            {
                Text = "Inverter Eixo Y",
                Checked = inv,
                ForeColor = UiTheme.Muted,
                Location = new Point(20, 72),
                Size = new Size(160, 24)
            };

            var lblSens = new Label { Text = "Sensibilidade:", ForeColor = UiTheme.Muted, Location = new Point(20, 108), AutoSize = true };
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

            var lblBtns = new Label { Text = "Botões Mouse:", ForeColor = UiTheme.Muted, Location = new Point(20, 146), AutoSize = true };
            _cmbBtns = new ComboBox
            {
                DropDownStyle = ComboBoxStyle.DropDownList,
                BackColor = UiTheme.Surface,
                ForeColor = UiTheme.Text,
                FlatStyle = FlatStyle.Flat,
                Location = new Point(118, 142),
                Size = new Size(282, 24)
            };
            _cmbBtns.Items.AddRange(new object[] { "Dir: Atirar | Esq: Mirar", "Esq: Atirar | Dir: Mirar", "Desativado" });
            _cmbBtns.SelectedIndex = btns == "lmb_shoot" ? 1 : (btns == "off" ? 2 : 0);

            var btnKeymap = new Button
            {
                Text = "Mapear Botões Xbox → Teclado...",
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
                    copy.Add(new KeyBindingItem { Id = it.Id, Label = it.Label, DefaultKey = it.DefaultKey, Key = it.Key, Accent = it.Accent });
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
                Text = "Cancelar",
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
                Text = "Aplicar",
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
        private const string RepoDir = @"C:\Users\gabri\Downloads\victory-klbf-lab39";
        private const string BackupsDir = @"C:\Users\gabri\Downloads\victory-backups";
        private const string DataDir = @"C:\Users\gabri\Downloads\victory-data";
        private const string ConfigPath = @"C:\Users\gabri\Downloads\victory-data\launcher-config.ini";
        private const string DiscordIdPath = @"C:\Users\gabri\Downloads\victory-data\discord_app_id.txt";
        private const string Ps1Path = @"C:\Users\gabri\Downloads\victory-klbf-lab39\run-klbf-lab39.ps1";
        private const string PrecompileBat = @"C:\Users\gabri\Downloads\GTA6.Xbox.v1.0.2.1-CYBERLEEK\PreCompilar-Shaders-KLBF-Lab39-AMD.bat";

        private readonly List<BuildEntry> _builds = new List<BuildEntry>();
        private readonly List<PresetEntry> _presets = new List<PresetEntry>();
        private readonly List<KeyBindingItem> _keymap = new List<KeyBindingItem>();

        private int _selectedBuild = 0;
        private int _selectedPreset = 0;
        private int _hoverBuild = -1;
        private int _hoverPreset = -1;
        // 0=Play, 1=Controls, 2=Shaders, 3=Logs, 4=DiscordCfg,
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
        private bool _cacheOk = false;
        private string _status = "";
        private float _scale = 1f;

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
            _cacheOk = File.Exists(Path.Combine(DataDir, "cache-klbf-lab39", "precompiled.ok"));

            InitDefaultKeymap();
            DiscoverBuilds();
            InitPresets();
            LoadConfig();

            ClientSize = new Size(S(560), S(548));
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
            _builds.Clear();
            string liveExe = Path.Combine(RepoDir, "target", "release", "victory.exe");
            if (File.Exists(liveExe))
            {
                _builds.Add(new BuildEntry
                {
                    Name = "Atual",
                    Meta = "build local",
                    ExePath = liveExe,
                    IsLatest = true
                });
            }

            if (Directory.Exists(BackupsDir))
            {
                string[] dirs = Directory.GetDirectories(BackupsDir, "motor1-v*");
                Array.Sort(dirs, StringComparer.OrdinalIgnoreCase);
                Array.Reverse(dirs); // newest version first (v1.2.0, v1.1.0-rc1, v1.0.0)
                foreach (string dir in dirs)
                {
                    string exe = Path.Combine(dir, "victory.exe");
                    if (!File.Exists(exe)) continue;
                    string folder = Path.GetFileName(dir);
                    string ver = folder.StartsWith("motor1-", StringComparison.OrdinalIgnoreCase)
                        ? folder.Substring("motor1-".Length)
                        : folder;
                    string meta = "";
                    string vf = Path.Combine(dir, "VERSION.txt");
                    if (File.Exists(vf))
                    {
                        try
                        {
                            foreach (string ln in File.ReadAllLines(vf))
                            {
                                if (ln.StartsWith("commit=", StringComparison.OrdinalIgnoreCase))
                                    meta = ln.Substring(7).Trim();
                            }
                        }
                        catch { }
                    }
                    _builds.Add(new BuildEntry { Name = ver, Meta = meta, ExePath = exe, IsLatest = false });
                }
            }

            if (_builds.Count == 0)
            {
                _builds.Add(new BuildEntry { Name = "Sem Build", Meta = "compile --release", ExePath = "", IsLatest = false });
            }
        }

        private void InitPresets()
        {
            _presets.Clear();
            _presets.Add(new PresetEntry { Title = "FSR 2.4 + FrameGen", Subtitle = "720p • Maior fluidez na RX 6600", Tag = "RECOMENDADO", TagColor = UiTheme.Green, Args = "-FrameGen -Res720p" });
            _presets.Add(new PresetEntry { Title = "FSR 2.4", Subtitle = "720p • Sem geração de quadros", Tag = "720p", TagColor = UiTheme.Cyan, Args = "-Res720p" });
            _presets.Add(new PresetEntry { Title = "Simple Ultraleve", Subtitle = "720p • Menor consumo de RAM e VRAM", Tag = "LEVE", TagColor = UiTheme.Blue, Args = "-SimpleUpscale -Res720p" });
            _presets.Add(new PresetEntry { Title = "FSR 2.4 Full HD", Subtitle = "1080p • Resolução nativa", Tag = "1080p", TagColor = UiTheme.Slate, Args = "" });
            _presets.Add(new PresetEntry { Title = "Modo Seguro", Subtitle = "720p • Flags conservadoras", Tag = "SAFE", TagColor = UiTheme.Amber, Args = "-Safe -Res720p" });
        }

        private void InitDefaultKeymap()
        {
            _keymap.Clear();
            Action<string, string, string, Color> add = (id, label, def, col) =>
                _keymap.Add(new KeyBindingItem { Id = id, Label = label, DefaultKey = def, Key = def, Accent = col });
            add("A", "Botão A", "ENTER", UiTheme.Green);
            add("B", "Botão B", "BACKSPACE", UiTheme.Red);
            add("X", "Botão X", "X", UiTheme.Blue);
            add("Y", "Botão Y", "Y", UiTheme.Amber);
            add("LB", "LB / L1", "Q", UiTheme.Cyan);
            add("RB", "RB / R1", "E", UiTheme.Cyan);
            add("LT", "LT / L2", "Z", UiTheme.Pink);
            add("RT", "RT / R2", "C", UiTheme.Pink);
            add("MENU", "Start / Menu", "ESC", UiTheme.Slate);
            add("VIEW", "Back / View", "TAB", UiTheme.Slate);
            add("LS", "L3", "F", UiTheme.Slate);
            add("RS", "R3", "H", UiTheme.Slate);
            add("UP", "D-Pad Cima", "UP", UiTheme.Cyan);
            add("DOWN", "D-Pad Baixo", "DOWN", UiTheme.Cyan);
            add("LEFT", "D-Pad Esq", "LEFT", UiTheme.Cyan);
            add("RIGHT", "D-Pad Dir", "RIGHT", UiTheme.Cyan);
            add("LY+", "Frente (LS ↑)", "W", UiTheme.Green);
            add("LY-", "Trás (LS ↓)", "S", UiTheme.Green);
            add("LX-", "Esq (LS ←)", "A", UiTheme.Green);
            add("LX+", "Dir (LS →)", "D", UiTheme.Green);
            add("RY+", "Câmera ↑", "I", UiTheme.Amber);
            add("RY-", "Câmera ↓", "K", UiTheme.Amber);
            add("RX-", "Câmera ←", "J", UiTheme.Amber);
            add("RX+", "Câmera →", "L", UiTheme.Amber);
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
                    if (k == "preset")
                    {
                        int idx;
                        if (int.TryParse(v, out idx) && idx >= 0 && idx < _presets.Count) _selectedPreset = idx;
                    }
                    else if (k == "build_version")
                    {
                        for (int i = 0; i < _builds.Count; i++)
                            if (string.Equals(_builds[i].Name, v, StringComparison.OrdinalIgnoreCase)) { _selectedBuild = i; break; }
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
                string buildName = (_selectedBuild >= 0 && _selectedBuild < _builds.Count) ? _builds[_selectedBuild].Name : "Atual";
                File.WriteAllLines(ConfigPath, new[]
                {
                    "preset=" + _selectedPreset,
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

            // Version selector row (up to 4 visible pills)
            int visibleBuilds = Math.Min(_builds.Count, 5);
            _buildRects = new Rectangle[visibleBuilds];
            int bGap = S(6);
            int bW = (w - bGap * (visibleBuilds - 1)) / Math.Max(1, visibleBuilds);
            int bY = S(84);
            for (int i = 0; i < visibleBuilds; i++)
                _buildRects[i] = new Rectangle(pad + i * (bW + bGap), bY, bW, S(34));

            // Preset rows (5 clean minimalist cards)
            _presetRects = new Rectangle[_presets.Count];
            int pY = bY + S(56);
            for (int i = 0; i < _presets.Count; i++)
            {
                _presetRects[i] = new Rectangle(pad, pY, w, S(42));
                pY += S(47);
            }

            // 2-column compact options grid
            int optY = pY + S(12);
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
            int btnY = optY + S(92);
            _btnControls = new Rectangle(pad, btnY, S(104), S(38));
            _btnShaders = new Rectangle(pad + S(110), btnY, S(84), S(38));
            _btnLogs = new Rectangle(pad + S(200), btnY, S(68), S(38));
            _btnPlay = new Rectangle(ClientSize.Width - pad - S(224), btnY, S(224), S(38));
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            Graphics g = e.Graphics;
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

            // Subtle 2px top accent bar
            using (var grad = new LinearGradientBrush(new Rectangle(0, 0, ClientSize.Width, S(2)), UiTheme.Pink, UiTheme.Cyan, 0f))
                g.FillRectangle(grad, 0, 0, ClientSize.Width, S(2));

            int pad = S(22);
            using (var fTitle = new Font("Segoe UI Semibold", 15f, FontStyle.Bold))
            using (var fSub = new Font("Segoe UI", 8.2f))
            using (var fSec = new Font("Segoe UI Semibold", 7.2f, FontStyle.Bold))
            using (var fItem = new Font("Segoe UI Semibold", 9.2f, FontStyle.Bold))
            using (var fItemSub = new Font("Segoe UI", 7.9f))
            using (var fPill = new Font("Segoe UI Semibold", 7.2f, FontStyle.Bold))
            using (var bTxt = new SolidBrush(UiTheme.Text))
            using (var bMut = new SolidBrush(UiTheme.Muted))
            using (var sfC = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center })
            {
                // Minimalist Header
                g.DrawString("GTA VI", fTitle, bTxt, pad, S(14));
                float wGta = g.MeasureString("GTA VI", fTitle).Width;
                using (var bCyan = new SolidBrush(UiTheme.Cyan))
                    g.DrawString("VICTORY", fTitle, bCyan, pad + wGta - S(2), S(14));

                g.DrawString("RX 6600 8 GB  •  Ryzen 5 4500  •  Motor 1", fSub, bMut, pad, S(42));

                // Cache status pill
                string cTxt = _cacheOk ? "SHADERS OK" : "CACHE FRIO";
                Color cCol = _cacheOk ? UiTheme.Green : UiTheme.Amber;
                int cW = (int)g.MeasureString(cTxt, fPill).Width + S(18);
                Rectangle cRect = new Rectangle(ClientSize.Width - pad - cW, S(20), cW, S(21));
                using (var p = UiTheme.RoundedRect(cRect, S(10)))
                using (var b = new SolidBrush(Color.FromArgb(24, cCol)))
                using (var pen = new Pen(Color.FromArgb(90, cCol), 1f))
                using (var bt = new SolidBrush(cCol))
                {
                    g.FillPath(b, p);
                    g.DrawPath(pen, p);
                    g.DrawString(cTxt, fPill, bt, cRect, sfC);
                }

                // Section 1: Version selector
                g.DrawString("VERSÃO DO MOTOR", fSec, bMut, pad, S(68));
                for (int i = 0; i < _buildRects.Length; i++)
                {
                    BuildEntry b = _builds[i];
                    Rectangle r = _buildRects[i];
                    bool sel = (i == _selectedBuild);
                    bool hov = (i == _hoverBuild);
                    Color bg = sel ? UiTheme.SurfaceActive : (hov ? UiTheme.SurfaceHover : UiTheme.Surface);
                    Color bd = sel ? UiTheme.Cyan : (hov ? UiTheme.BorderHover : UiTheme.Border);

                    using (var path = UiTheme.RoundedRect(r, S(7)))
                    using (var bBg = new SolidBrush(bg))
                    using (var pen = new Pen(bd, sel ? 1.4f : 1f))
                    {
                        g.FillPath(bBg, path);
                        g.DrawPath(pen, path);
                    }
                    using (var bName = new SolidBrush(sel ? UiTheme.Cyan : UiTheme.Text))
                    {
                        Rectangle nr = string.IsNullOrEmpty(b.Meta) ? r : new Rectangle(r.X, r.Y + S(3), r.Width, S(16));
                        g.DrawString(b.Name, fPill, bName, nr, sfC);
                    }
                    if (!string.IsNullOrEmpty(b.Meta))
                    {
                        Rectangle mr = new Rectangle(r.X, r.Y + S(18), r.Width, S(13));
                        using (var fMicro = new Font("Segoe UI", 6.8f))
                            g.DrawString(b.Meta, fMicro, bMut, mr, sfC);
                    }
                }

                // Section 2: Presets
                g.DrawString("PERFIL GRÁFICO", fSec, bMut, pad, S(124));
                for (int i = 0; i < _presetRects.Length; i++)
                {
                    PresetEntry pItem = _presets[i];
                    Rectangle r = _presetRects[i];
                    bool sel = (i == _selectedPreset);
                    bool hov = (i == _hoverPreset);

                    Color bg = sel ? UiTheme.SurfaceActive : (hov ? UiTheme.SurfaceHover : UiTheme.Surface);
                    Color bd = sel ? UiTheme.Cyan : (hov ? UiTheme.BorderHover : UiTheme.Border);
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

                    g.DrawString(pItem.Title, fItem, bTxt, r.X + S(13), r.Y + S(5));
                    g.DrawString(pItem.Subtitle, fItemSub, bMut, r.X + S(13), r.Y + S(23));

                    int tw = (int)g.MeasureString(pItem.Tag, fPill).Width + S(14);
                    Rectangle tr = new Rectangle(r.Right - tw - S(10), r.Y + S(11), tw, S(20));
                    using (var tp = UiTheme.RoundedRect(tr, S(9)))
                    using (var tb = new SolidBrush(Color.FromArgb(26, pItem.TagColor)))
                    using (var tt = new SolidBrush(pItem.TagColor))
                    {
                        g.FillPath(tb, tp);
                        g.DrawString(pItem.Tag, fPill, tt, tr, sfC);
                    }
                }

                // Section 3: Compact 2-column toggles
                DrawCheck(g, _chkUnified, _unifiedMemory, "Memória Unificada (Tiled)", UiTheme.Cyan, _hoverCtrl == 10);
                DrawCheck(g, _chkBoost, _cpuGpuBoost, "Boost CPU↔GPU (v1.2)", UiTheme.Green, _hoverCtrl == 11);
                DrawCheck(g, _chkLite, _liteGraphics, "Modo Gráfico Leve", UiTheme.Amber, _hoverCtrl == 12);
                DrawCheck(g, _chkHud, _enableHud, "HUD de FPS", UiTheme.Cyan, _hoverCtrl == 13);

                string rpcLbl = _discordRpc && string.IsNullOrEmpty(_discordAppId) ? "Discord RPC (sem ID)" : "Discord Rich Presence";
                DrawCheck(g, _chkDiscord, _discordRpc, rpcLbl, UiTheme.Blurple, _hoverCtrl == 14);
                DrawMiniPill(g, _btnDiscordCfg, string.IsNullOrEmpty(_discordAppId) ? "Set ID" : "ID ✓", UiTheme.Blurple, _hoverCtrl == 4, sfC);

                DrawCheck(g, _chkClose, _closeOnLaunch, "Fechar ao Jogar", UiTheme.Slate, _hoverCtrl == 15);

                // Section 4: Action bar
                DrawActionBtn(g, _btnControls, _mouseCamera ? "Controles •" : "Controles", false, _hoverCtrl == 1, sfC);
                DrawActionBtn(g, _btnShaders, "Shaders", false, _hoverCtrl == 2, sfC);
                DrawActionBtn(g, _btnLogs, "Logs", false, _hoverCtrl == 3, sfC);

                string verTag = (_selectedBuild >= 0 && _selectedBuild < _builds.Count) ? _builds[_selectedBuild].Name : "";
                DrawActionBtn(g, _btnPlay, "▶  JOGAR  (" + verTag + ")", true, _hoverCtrl == 0, sfC);

                if (!string.IsNullOrEmpty(_status))
                {
                    using (var bSt = new SolidBrush(UiTheme.Amber))
                        g.DrawString(_status, fSub, bSt, pad, ClientSize.Height - S(20));
                }
            }
        }

        private void DrawCheck(Graphics g, Rectangle r, bool on, string label, Color accent, bool hov)
        {
            Rectangle box = new Rectangle(r.X, r.Y + S(4), S(14), S(14));
            using (var bp = UiTheme.RoundedRect(box, S(4)))
            using (var bf = new SolidBrush(on ? accent : UiTheme.Surface))
            using (var pen = new Pen(on ? accent : (hov ? UiTheme.BorderHover : UiTheme.Border), 1.2f))
            {
                g.FillPath(bf, bp);
                g.DrawPath(pen, bp);
            }
            if (on)
            {
                using (var p = new Pen(UiTheme.Bg, 1.9f))
                {
                    g.DrawLine(p, box.X + S(3), box.Y + S(7), box.X + S(6), box.Y + S(10));
                    g.DrawLine(p, box.X + S(6), box.Y + S(10), box.X + S(11), box.Y + S(4));
                }
            }
            using (var f = new Font("Segoe UI", 8.4f))
            using (var b = new SolidBrush(on || hov ? UiTheme.Text : UiTheme.Muted))
                g.DrawString(label, f, b, box.Right + S(7), r.Y + S(3));
        }

        private void DrawMiniPill(Graphics g, Rectangle r, string txt, Color col, bool hov, StringFormat sf)
        {
            using (var p = UiTheme.RoundedRect(r, S(5)))
            using (var bf = new SolidBrush(hov ? Color.FromArgb(46, col) : Color.FromArgb(24, col)))
            using (var pen = new Pen(Color.FromArgb(hov ? 180 : 100, col), 1f))
            using (var f = new Font("Segoe UI Semibold", 7.3f, FontStyle.Bold))
            using (var bt = new SolidBrush(col))
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
            using (var f = new Font("Segoe UI Semibold", primary ? 9.2f : 8.4f, FontStyle.Bold))
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
            else if (_btnDiscordCfg.Contains(e.Location)) _hoverCtrl = 4;
            else if (_chkUnified.Contains(e.Location)) _hoverCtrl = 10;
            else if (_chkBoost.Contains(e.Location)) _hoverCtrl = 11;
            else if (_chkLite.Contains(e.Location)) _hoverCtrl = 12;
            else if (_chkHud.Contains(e.Location)) _hoverCtrl = 13;
            else if (_chkDiscord.Contains(e.Location)) _hoverCtrl = 14;
            else if (_chkClose.Contains(e.Location)) _hoverCtrl = 15;

            Cursor = (_hoverBuild >= 0 || _hoverPreset >= 0 || _hoverCtrl >= 0) ? Cursors.Hand : Cursors.Default;
            if (ob != _hoverBuild || op != _hoverPreset || oc != _hoverCtrl) Invalidate();
        }

        private void OnMouseDownCanvas(object sender, MouseEventArgs e)
        {
            if (e.Button != MouseButtons.Left) return;

            for (int i = 0; i < _buildRects.Length; i++)
            {
                if (_buildRects[i].Contains(e.Location))
                {
                    _selectedBuild = i;
                    _status = "Versão selecionada: " + _builds[i].Name;
                    SaveConfig();
                    Invalidate();
                    return;
                }
            }
            for (int i = 0; i < _presetRects.Length; i++)
            {
                if (_presetRects[i].Contains(e.Location))
                {
                    _selectedPreset = i;
                    _status = "";
                    SaveConfig();
                    Invalidate();
                    return;
                }
            }

            if (_chkUnified.Contains(e.Location)) { _unifiedMemory = !_unifiedMemory; SaveConfig(); Invalidate(); }
            else if (_chkBoost.Contains(e.Location)) { _cpuGpuBoost = !_cpuGpuBoost; SaveConfig(); Invalidate(); }
            else if (_chkLite.Contains(e.Location)) { _liteGraphics = !_liteGraphics; SaveConfig(); Invalidate(); }
            else if (_chkHud.Contains(e.Location)) { _enableHud = !_enableHud; SaveConfig(); Invalidate(); }
            else if (_chkClose.Contains(e.Location)) { _closeOnLaunch = !_closeOnLaunch; SaveConfig(); Invalidate(); }
            else if (_chkDiscord.Contains(e.Location))
            {
                _discordRpc = !_discordRpc;
                if (_discordRpc && string.IsNullOrEmpty(_discordAppId)) OpenDiscordConfig();
                else { SaveConfig(); Invalidate(); }
            }
            else if (_btnDiscordCfg.Contains(e.Location)) { OpenDiscordConfig(); }
            else if (_btnControls.Contains(e.Location)) { OpenControlsDialog(); }
            else if (_btnShaders.Contains(e.Location))
            {
                if (File.Exists(PrecompileBat))
                    Process.Start(new ProcessStartInfo { FileName = PrecompileBat, WorkingDirectory = Path.GetDirectoryName(PrecompileBat), UseShellExecute = true });
                else { _status = "Script de pré-compilação não encontrado."; Invalidate(); }
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
            {
                if (_presetRects[i].Contains(e.Location)) { _selectedPreset = i; SaveConfig(); LaunchSelected(); return; }
            }
        }

        private void OnKeyDownCanvas(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Up && _selectedPreset > 0) { _selectedPreset--; SaveConfig(); Invalidate(); }
            else if (e.KeyCode == Keys.Down && _selectedPreset < _presets.Count - 1) { _selectedPreset++; SaveConfig(); Invalidate(); }
            else if (e.KeyCode == Keys.Left && _selectedBuild > 0) { _selectedBuild--; SaveConfig(); Invalidate(); }
            else if (e.KeyCode == Keys.Right && _selectedBuild < _buildRects.Length - 1) { _selectedBuild++; SaveConfig(); Invalidate(); }
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
                    _status = string.IsNullOrEmpty(_discordAppId) ? "Discord App ID limpo." : ("Discord RPC salvo (ID " + _discordAppId + ").");
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
                    _status = "Controles salvos.";
                    SaveConfig();
                    Invalidate();
                }
            }
        }

        private void LaunchSelected()
        {
            if (_selectedBuild < 0 || _selectedBuild >= _builds.Count) return;
            BuildEntry build = _builds[_selectedBuild];
            if (string.IsNullOrEmpty(build.ExePath) || !File.Exists(build.ExePath))
            {
                _status = "Executável da versão " + build.Name + " não encontrado.";
                Invalidate();
                return;
            }
            if (!File.Exists(Ps1Path))
            {
                _status = "run-klbf-lab39.ps1 não encontrado.";
                Invalidate();
                return;
            }

            PresetEntry preset = _presets[_selectedPreset];
            SaveConfig();

            var parts = new List<string>();
            if (!string.IsNullOrEmpty(preset.Args)) parts.Add(preset.Args);
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

            try
            {
                Process.Start(new ProcessStartInfo
                {
                    FileName = "powershell.exe",
                    Arguments = "-NoProfile -ExecutionPolicy Bypass -File \"" + Ps1Path + "\" " + string.Join(" ", parts.ToArray()),
                    WorkingDirectory = RepoDir,
                    UseShellExecute = true
                });

                if (_closeOnLaunch) Close();
                else { _status = "Iniciado: " + build.Name + " • " + preset.Title; TrimMemory(); Invalidate(); }
            }
            catch (Exception ex)
            {
                _status = "Erro: " + ex.Message;
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
