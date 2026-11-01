// The Java half of every Kova app (PLAN Part 4 "Android"): real Android views for a Kova view.
// It is the same job as `kovaPage` in runtime/web.js, from the same JSON (runtime/view.rs).
//
// The program runs natively on its own thread. `call` is the only link: this class waits in
// `take` for what the program asks, and `give`s back what happened.
//
// Compiled by runtime/android/make.sh into classes.dex, which is kept in the repository: neither
// building Kova nor using it needs Android's tools.
package dev.kova;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Context;
import android.content.Intent;
import android.content.res.Configuration;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.Typeface;
import android.media.MediaPlayer;
import android.net.Uri;
import android.opengl.GLES30;
import android.opengl.GLSurfaceView;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.util.DisplayMetrics;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.SubMenu;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.inputmethod.EditorInfo;
import android.widget.AdapterView;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.HorizontalScrollView;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.Spinner;
import android.widget.TableLayout;
import android.widget.TableRow;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.SocketTimeoutException;
import java.net.URL;
import java.net.UnknownHostException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.FloatBuffer;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

import javax.microedition.khronos.egl.EGLConfig;
import javax.microedition.khronos.opengles.GL10;

public class Main extends Activity {
    static {
        System.loadLibrary("kova");
    }

    static final int START = 0, TAKE = 1, GIVE = 2;
    static final long RESIZED = 1L << 40;
    static final long SHADER_FAILED = -1_000_000_000L;
    static final int PICK_FILE = 1;

    static native byte[] call(int op, byte[] data);

    static boolean started;

    final Handler ui = new Handler(Looper.getMainLooper());
    FrameLayout window;
    ScrollView consoleBox;
    LinearLayout consoleColumn;
    TextView console;
    EditText consoleLine;
    float density;

    boolean showing, ended;
    String shape;
    View root;
    JSONObject waiting;
    String menusStamp = "";
    JSONArray menuBar = new JSONArray();
    final Map<Integer, int[]> rooms = new HashMap<>();
    final Map<String, Runnable> clocks = new HashMap<>();
    final Map<String, Bitmap> images = new HashMap<>();

    static class Style {
        Integer color;
        double size;
        boolean bold;

        Style over(JSONArray n) throws JSONException {
            Style s = new Style();
            s.color = n.isNull(1) ? color : Integer.valueOf(n.getInt(1));
            s.size = n.getDouble(2) != 0 ? n.getDouble(2) : size;
            s.bold = n.getBoolean(3) || bold;
            return s;
        }
    }

    @Override
    protected void onCreate(Bundle saved) {
        boolean night = (getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
        setTheme(night ? android.R.style.Theme_DeviceDefault : android.R.style.Theme_DeviceDefault_Light);
        super.onCreate(saved);
        density = getResources().getDisplayMetrics().density;
        window = new FrameLayout(this);
        // Android 15 draws apps under its bars: the system says how much room they take.
        window.setOnApplyWindowInsetsListener((v, insets) -> {
            int left, top, right, bottom;
            if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets i = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.ime());
                left = i.left;
                top = i.top;
                right = i.right;
                bottom = i.bottom;
            } else {
                left = insets.getSystemWindowInsetLeft();
                top = insets.getSystemWindowInsetTop();
                right = insets.getSystemWindowInsetRight();
                bottom = insets.getSystemWindowInsetBottom();
            }
            v.setPadding(left, top, right, bottom);
            return insets;
        });
        console = new TextView(this);
        console.setTypeface(Typeface.MONOSPACE);
        console.setTextIsSelectable(true);
        console.setDefaultFocusHighlightEnabled(false);
        text(console, new Style());
        console.setTypeface(Typeface.MONOSPACE);
        int gap = dp(8);
        console.setPadding(gap, gap, gap, gap);
        consoleLine = new EditText(this);
        consoleLine.setSingleLine(true);
        consoleLine.setImeOptions(EditorInfo.IME_ACTION_DONE);
        consoleLine.setVisibility(View.GONE);
        consoleLine.setOnEditorActionListener((v, action, e) -> {
            String line = consoleLine.getText().toString();
            consoleLine.setText("");
            consoleLine.setVisibility(View.GONE);
            console.append(line + "\n");
            reply(line.getBytes(StandardCharsets.UTF_8));
            return true;
        });
        consoleColumn = new LinearLayout(this);
        consoleColumn.setOrientation(LinearLayout.VERTICAL);
        consoleColumn.addView(console);
        consoleColumn.addView(consoleLine);
        consoleBox = new ScrollView(this);
        consoleBox.addView(consoleColumn);
        window.addView(consoleBox);
        setContentView(window);
        window.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, orr, ob) -> {
            if (r - l != orr - ol || b - t != ob - ot) {
                if (showing) hear(RESIZED, "");
            }
        });
        if (started) return;
        started = true;
        String folder = getFilesDir().getPath() + "\n" + Locale.getDefault().getLanguage();
        call(START, folder.getBytes(StandardCharsets.UTF_8));
        Thread reader = new Thread(this::listen, "kova-reader");
        reader.setDaemon(true);
        reader.start();
    }

    @Override
    protected void onDestroy() {
        super.onDestroy();
        if (isFinishing()) {
            if (showing && !ended) hear(-1, "");
            else System.exit(0);
        }
    }

    int dp(double n) {
        return (int) Math.round(n * density);
    }

    // ---- What the program asks ----

    void listen() {
        while (true) {
            byte[] m = call(TAKE, null);
            byte[] content = new byte[m.length - 1];
            System.arraycopy(m, 1, content, 0, content.length);
            switch (m[0]) {
                case 'P': {
                    String text = new String(content, StandardCharsets.UTF_8);
                    ui.post(() -> {
                        console.append(text);
                        consoleBox.post(() -> consoleBox.fullScroll(View.FOCUS_DOWN));
                    });
                    break;
                }
                case 'V':
                    view(new String(content, StandardCharsets.UTF_8));
                    break;
                case 'X': {
                    int code = Integer.parseInt(new String(content, StandardCharsets.UTF_8));
                    ui.post(() -> ended(code));
                    break;
                }
                case 'R':
                    ui.post(() -> {
                        if (!showing) consoleLine.setVisibility(View.VISIBLE);
                        consoleLine.requestFocus();
                    });
                    break;
                case 'A':
                    ui.post(() -> ask(new String(content, StandardCharsets.UTF_8)));
                    break;
                case 'S':
                    ui.post(() -> play(content));
                    break;
                case 'W':
                    new Thread(() -> fetch(content)).start();
                    break;
                default:
                    break;
            }
        }
    }

    void reply(byte[] answer) {
        byte[] m = new byte[answer.length + 1];
        m[0] = 'R';
        System.arraycopy(answer, 0, m, 1, answer.length);
        call(GIVE, m);
    }

    void reply(String answer) {
        reply(answer.getBytes(StandardCharsets.UTF_8));
    }

    /** The program is over: a console stays to be read, a window closes, an error shows. */
    void ended(int code) {
        ended = true;
        if (code != 0 && showing) {
            window.removeAllViews();
            window.addView(consoleBox);
            showing = false;
        } else if (showing || isFinishing()) {
            System.exit(0);
        }
    }

    /** Only the last view counts: a program can draw faster than the screen. */
    void view(String json) {
        JSONObject o;
        try {
            o = new JSONObject(json);
        } catch (JSONException e) {
            return;
        }
        boolean first;
        synchronized (this) {
            first = waiting == null;
            waiting = o;
        }
        if (!first) return;
        ui.post(() -> {
            JSONObject latest;
            synchronized (this) {
                latest = waiting;
                waiting = null;
            }
            try {
                render(latest);
            } catch (JSONException e) {
                throw new RuntimeException(e);
            }
        });
    }

    // ---- Events ----

    void hear(long answer, String words) {
        StringBuilder rooms = new StringBuilder();
        for (Map.Entry<Integer, int[]> e : this.rooms.entrySet()) {
            int[] r = e.getValue();
            rooms.append(e.getKey()).append(' ').append(r[0]).append(' ').append(r[1]).append(',');
        }
        DisplayMetrics m = getResources().getDisplayMetrics();
        int w = (int) (window.getWidth() / density), h = (int) (window.getHeight() / density);
        int sw = (int) (m.widthPixels / density), sh = (int) (m.heightPixels / density);
        String areas = "0 0 " + w + " " + h + " 0 0 " + sw + " " + sh;
        String event = "E" + answer + "\n" + areas + "\n" + rooms + "\n" + words;
        call(GIVE, event.getBytes(StandardCharsets.UTF_8));
    }

    static long changed(long k) {
        return -2 - k;
    }

    // ---- Views ----

    void render(JSONObject o) throws JSONException {
        if (ended) return;
        setTitle(o.getString("title"));
        JSONArray acts = o.getJSONArray("acts");
        for (int i = 0; i < acts.length(); i++) {
            int act = acts.getInt(i);
            if (act == 0 && getActionBar() != null) {
                getActionBar().hide();
                window.requestApplyInsets();
            }
            if (act == 1) {
                finish();
                return;
            }
            if (act == 2) moveTaskToBack(true);
            if (act == 5 && Build.VERSION.SDK_INT >= 30) window.getWindowInsetsController().hide(WindowInsets.Type.systemBars());
        }
        JSONArray bar = o.getJSONArray("menus");
        if (!bar.toString().equals(menusStamp)) {
            menusStamp = bar.toString();
            menuBar = bar;
            invalidateOptionsMenu();
        }
        JSONArray view = o.getJSONArray("view");
        Style s = new Style();
        String seen = o.getString("shape");
        if (seen.equals(shape)) {
            update(root, view, s);
        } else {
            shape = seen;
            root = build(view, s);
            window.removeAllViews();
            FrameLayout.LayoutParams all = new FrameLayout.LayoutParams(-1, -1);
            int gap = dp(8);
            if (fills(view, 1)) {
                root.setPadding(gap, gap, gap, gap);
                window.addView(root, all);
            } else {
                ScrollView scroll = new ScrollView(this);
                scroll.setPadding(gap, gap, gap, gap);
                scroll.addView(root);
                window.addView(scroll, all);
            }
            showing = true;
        }
        Set<String> ticking = new HashSet<>();
        clocksOf(view, ticking);
        for (String key : ticking) {
            if (clocks.containsKey(key)) continue;
            String[] p = key.split(":");
            long ms = Long.parseLong(p[0]), k = Long.parseLong(p[1]);
            Runnable tick = new Runnable() {
                @Override
                public void run() {
                    hear(k, "");
                    ui.postDelayed(this, ms);
                }
            };
            clocks.put(key, tick);
            ui.postDelayed(tick, ms);
        }
        clocks.keySet().removeIf(key -> {
            if (ticking.contains(key)) return false;
            ui.removeCallbacks(clocks.get(key));
            return true;
        });
    }

    void clocksOf(JSONArray n, Set<String> found) throws JSONException {
        switch (n.getString(0)) {
            case "E":
                found.add(n.getLong(1) + ":" + n.getLong(2));
                clocksOf(n.getJSONArray(3), found);
                break;
            case "C":
            case "R":
                for (int i = 0; i < n.getJSONArray(2).length(); i++) clocksOf(n.getJSONArray(2).getJSONArray(i), found);
                break;
            case "M":
                clocksOf(n.getJSONArray(2), found);
                break;
            case "S":
                clocksOf(n.getJSONArray(4), found);
                break;
            default:
                break;
        }
    }

    /** Whether `n` holds a canvas taking all the room there is, in width (0) or height (1). */
    boolean fills(JSONArray n, int side) throws JSONException {
        switch (n.getString(0)) {
            case "D":
                return n.getInt(1 + side) == 0;
            case "E":
                return fills(n.getJSONArray(3), side);
            case "S":
                return fills(n.getJSONArray(4), side);
            case "M":
                return fills(n.getJSONArray(2), side);
            case "C":
            case "R": {
                JSONArray parts = n.getJSONArray(2);
                for (int i = 0; i < parts.length(); i++) {
                    if (fills(parts.getJSONArray(i), side)) return true;
                }
                return false;
            }
            default:
                return false;
        }
    }

    void text(TextView v, Style s) {
        if (s.color != null) {
            v.setTextColor(0xFF000000 | s.color);
        } else {
            android.content.res.TypedArray main = obtainStyledAttributes(new int[] {android.R.attr.textColorPrimary});
            v.setTextColor(main.getColor(0, Color.BLACK));
            main.recycle();
        }
        if (s.size != 0) v.setTextSize(TypedValue.COMPLEX_UNIT_DIP, (float) s.size);
        v.setTypeface(null, s.bold ? Typeface.BOLD : Typeface.NORMAL);
    }

    View build(JSONArray n, Style s) throws JSONException {
        View v;
        switch (n.getString(0)) {
            case "T":
                v = new TextView(this);
                break;
            case "B": {
                Button b = new Button(this);
                b.setAllCaps(false);
                b.setOnClickListener(x -> hear((long) x.getTag(), ""));
                v = b;
                break;
            }
            case "I": {
                EditText e = new EditText(this);
                e.setSingleLine(true);
                e.setInputType(InputType.TYPE_CLASS_TEXT);
                e.addTextChangedListener(new TextWatcher() {
                    @Override
                    public void beforeTextChanged(CharSequence t, int a, int b, int c) {}

                    @Override
                    public void onTextChanged(CharSequence t, int a, int b, int c) {}

                    @Override
                    public void afterTextChanged(Editable t) {
                        if (e.getTag(R_APPLYING) == null) hear(changed((long) e.getTag()), t.toString());
                    }
                });
                v = e;
                break;
            }
            case "X": {
                CheckBox c = new CheckBox(this);
                c.setOnCheckedChangeListener((x, on) -> {
                    if (x.getTag(R_APPLYING) == null) hear(changed((long) x.getTag()), on ? "on" : "off");
                });
                v = c;
                break;
            }
            case "W": {
                SeekBar bar = new SeekBar(this);
                bar.setMinimumWidth(dp(200));
                bar.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                    @Override
                    public void onProgressChanged(SeekBar b, int value, boolean byUser) {
                        if (byUser) hear(changed((long) b.getTag()), String.valueOf(value));
                    }

                    @Override
                    public void onStartTrackingTouch(SeekBar b) {}

                    @Override
                    public void onStopTrackingTouch(SeekBar b) {}
                });
                v = bar;
                break;
            }
            case "O": {
                Spinner spinner = new Spinner(this);
                List<String> items = strings(n.getJSONArray(1));
                spinner.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item, items));
                spinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
                    @Override
                    public void onItemSelected(AdapterView<?> p, View x, int i, long id) {
                        Object was = spinner.getTag(R_APPLYING);
                        if (was == null || (int) was != i) hear(changed((long) spinner.getTag()), String.valueOf(i));
                    }

                    @Override
                    public void onNothingSelected(AdapterView<?> p) {}
                });
                v = spinner;
                break;
            }
            case "G":
                v = table(n);
                break;
            case "P":
                v = new ImageView(this);
                break;
            case "C":
            case "R": {
                LinearLayout l = new LinearLayout(this);
                boolean row = n.getString(0).equals("R");
                l.setOrientation(row ? LinearLayout.HORIZONTAL : LinearLayout.VERTICAL);
                JSONArray parts = n.getJSONArray(2);
                for (int i = 0; i < parts.length(); i++) {
                    JSONArray part = parts.getJSONArray(i);
                    boolean w = fills(part, 0), h = fills(part, 1);
                    LinearLayout.LayoutParams p = row
                        ? new LinearLayout.LayoutParams(w ? 0 : -2, h ? -1 : -2, w ? 1 : 0)
                        : new LinearLayout.LayoutParams(w ? -1 : -2, h ? 0 : -2, h ? 1 : 0);
                    if (i > 0) {
                        if (row) p.leftMargin = dp(8);
                        else p.topMargin = dp(8);
                    }
                    l.addView(build(part, s), p);
                }
                v = l;
                break;
            }
            case "M": {
                FrameLayout box = new FrameLayout(this);
                JSONArray inner = n.getJSONArray(2);
                box.addView(build(inner, s), new FrameLayout.LayoutParams(fills(inner, 0) ? -1 : -2, fills(inner, 1) ? -1 : -2));
                v = box;
                break;
            }
            case "S":
                return build(n.getJSONArray(4), s.over(n));
            case "E":
                return build(n.getJSONArray(3), s);
            case "D": {
                JSONArray ops = n.getJSONArray(6);
                boolean gpu = ops.length() > 0 && ops.getJSONArray(0).getString(0).equals("g");
                v = gpu ? new Scene(this, n.getInt(3)) : new Board(this, n.getInt(3));
                break;
            }
            default:
                throw new JSONException("unknown view " + n.getString(0));
        }
        update(v, n, s);
        return v;
    }

    /** A tag of Kova's own, unlike the number each view keeps in the plain tag. */
    static final int R_APPLYING = 0x7F010001;

    static List<String> strings(JSONArray a) throws JSONException {
        List<String> out = new ArrayList<>();
        for (int i = 0; i < a.length(); i++) out.add(a.getString(i));
        return out;
    }

    View table(JSONArray n) throws JSONException {
        TableLayout t = new TableLayout(this);
        t.setPadding(0, 0, 0, 0);
        JSONArray headers = n.getJSONArray(1), rows = n.getJSONArray(2);
        if (headers.length() > 0) {
            TableRow row = new TableRow(this);
            for (String h : strings(headers)) {
                TextView cell = cell(h);
                cell.setTypeface(null, Typeface.BOLD);
                row.addView(cell);
            }
            t.addView(row);
        }
        for (int i = 0; i < rows.length(); i++) {
            TableRow row = new TableRow(this);
            for (String c : strings(rows.getJSONArray(i))) row.addView(cell(c));
            int number = i;
            row.setOnClickListener(x -> hear(changed((long) t.getTag()), String.valueOf(number)));
            t.addView(row);
        }
        HorizontalScrollView scroll = new HorizontalScrollView(this);
        scroll.addView(t);
        return scroll;
    }

    TextView cell(String text) {
        TextView v = new TextView(this);
        v.setText(text);
        text(v, new Style());
        v.setPadding(dp(8), dp(8), dp(8), dp(8));
        return v;
    }

    void update(View v, JSONArray n, Style s) throws JSONException {
        switch (n.getString(0)) {
            case "T": {
                TextView t = (TextView) v;
                if (!t.getText().toString().equals(n.getString(1))) t.setText(n.getString(1));
                text(t, s);
                break;
            }
            case "B": {
                Button b = (Button) v;
                b.setTag(n.getLong(2));
                if (!b.getText().toString().equals(n.getString(1))) b.setText(n.getString(1));
                break;
            }
            case "I": {
                EditText e = (EditText) v;
                e.setTag(n.getLong(2));
                if (!e.getText().toString().equals(n.getString(1))) {
                    e.setTag(R_APPLYING, true);
                    e.setText(n.getString(1));
                    e.setSelection(e.length());
                    e.setTag(R_APPLYING, null);
                }
                text(e, s);
                break;
            }
            case "X": {
                CheckBox c = (CheckBox) v;
                c.setTag(n.getLong(3));
                c.setText(n.getString(1));
                c.setTag(R_APPLYING, true);
                c.setChecked(n.getBoolean(2));
                c.setTag(R_APPLYING, null);
                break;
            }
            case "W": {
                SeekBar bar = (SeekBar) v;
                bar.setTag(n.getLong(4));
                bar.setMin(n.getInt(2));
                bar.setMax(n.getInt(3));
                bar.setProgress(n.getInt(1));
                break;
            }
            case "O": {
                Spinner spinner = (Spinner) v;
                spinner.setTag(n.getLong(3));
                spinner.setTag(R_APPLYING, n.getInt(2));
                spinner.setSelection(n.getInt(2));
                break;
            }
            case "G": {
                TableLayout t = (TableLayout) ((HorizontalScrollView) v).getChildAt(0);
                t.setTag(n.getLong(4));
                int first = n.getJSONArray(1).length() > 0 ? 1 : 0;
                for (int i = first; i < t.getChildCount(); i++) {
                    t.getChildAt(i).setBackgroundColor(i - first == n.getInt(3) ? 0x40808080 : Color.TRANSPARENT);
                }
                break;
            }
            case "P": {
                ImageView image = (ImageView) v;
                image.setAdjustViewBounds(true);
                Bitmap b = picture(n.getString(1));
                if (b != null) image.setImageBitmap(b);
                if (n.getDouble(2) != 0) image.setMaxWidth(dp(n.getDouble(2)));
                break;
            }
            case "C":
            case "R": {
                LinearLayout l = (LinearLayout) v;
                int side = new int[] {Gravity.START, Gravity.CENTER, Gravity.END}[n.getInt(1)];
                l.setGravity(n.getString(0).equals("C") ? side & Gravity.HORIZONTAL_GRAVITY_MASK : side & Gravity.HORIZONTAL_GRAVITY_MASK);
                if (n.getInt(1) != 0 && n.getString(0).equals("R")) l.setGravity(side | Gravity.CENTER_VERTICAL);
                if (n.getInt(1) != 0 && l.getLayoutParams() != null) l.getLayoutParams().width = -1;
                JSONArray parts = n.getJSONArray(2);
                for (int i = 0; i < parts.length(); i++) update(l.getChildAt(i), parts.getJSONArray(i), s);
                break;
            }
            case "M": {
                int padding = dp(n.getDouble(1));
                v.setPadding(padding, padding, padding, padding);
                update(((FrameLayout) v).getChildAt(0), n.getJSONArray(2), s);
                break;
            }
            case "S":
                update(v, n.getJSONArray(4), s.over(n));
                break;
            case "E":
                update(v, n.getJSONArray(3), s);
                break;
            case "D": {
                JSONArray ops = n.getJSONArray(6);
                if (n.getInt(1) != 0 || n.getInt(2) != 0) {
                    ViewGroup.LayoutParams p = v.getLayoutParams();
                    if (p == null) {
                        p = new ViewGroup.LayoutParams(-2, -2);
                    }
                    if (n.getInt(1) != 0) p.width = dp(n.getDouble(1));
                    if (n.getInt(2) != 0) p.height = dp(n.getDouble(2));
                    v.setLayoutParams(p);
                }
                if (v instanceof Board) ((Board) v).paint(n, ops);
                else ((Scene) v).paint(n, ops);
                break;
            }
            default:
                break;
        }
    }

    Bitmap picture(String path) {
        if (!images.containsKey(path)) images.put(path, BitmapFactory.decodeFile(path));
        return images.get(path);
    }

    // ---- Menus: the bar's three dots ----

    final Map<Integer, Long> menuAnswers = new HashMap<>();

    @Override
    public boolean onPrepareOptionsMenu(Menu menu) {
        menu.clear();
        menuAnswers.clear();
        try {
            int id = 1;
            for (int i = 0; i < menuBar.length(); i++) {
                JSONObject m = menuBar.getJSONObject(i);
                SubMenu sub = menu.addSubMenu(m.getString("title"));
                JSONArray items = m.getJSONArray("items");
                for (int k = 0; k < items.length(); k++) {
                    JSONArray item = items.getJSONArray(k);
                    if (item.isNull(1)) continue;
                    sub.add(0, id, 0, item.getString(0));
                    menuAnswers.put(id++, item.getLong(1));
                }
            }
        } catch (JSONException e) {
            return false;
        }
        return menu.size() > 0;
    }

    @Override
    public boolean onOptionsItemSelected(MenuItem item) {
        Long answer = menuAnswers.get(item.getItemId());
        if (answer == null) return false;
        hear(answer, "");
        return true;
    }

    // ---- Canvas ----

    /** Touch and keys, for a canvas of either kind. */
    class Surface {
        final int number;
        Long mouse, key;

        Surface(int number) {
            this.number = number;
        }

        void options(JSONArray n) throws JSONException {
            mouse = n.isNull(4) ? null : Long.valueOf(n.getLong(4));
            key = n.isNull(5) ? null : Long.valueOf(n.getLong(5));
        }

        void sized(int w, int h) {
            rooms.put(number, new int[] {Math.round(w / density), Math.round(h / density)});
            hear(RESIZED, "");
        }

        boolean touch(MotionEvent e) {
            if (mouse == null) return false;
            String where = Math.round(e.getX() / density) + " " + Math.round(e.getY() / density);
            switch (e.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    hear(changed(mouse), "press " + where + " 1");
                    return true;
                case MotionEvent.ACTION_UP:
                    hear(changed(mouse), "release " + where + " 1");
                    return true;
                case MotionEvent.ACTION_MOVE:
                    hear(changed(mouse), "move " + where + " 0");
                    return true;
                default:
                    return false;
            }
        }

        boolean press(int code, KeyEvent e) {
            if (key == null) return false;
            String name;
            switch (code) {
                case KeyEvent.KEYCODE_DPAD_LEFT: name = "Left"; break;
                case KeyEvent.KEYCODE_DPAD_RIGHT: name = "Right"; break;
                case KeyEvent.KEYCODE_DPAD_UP: name = "Up"; break;
                case KeyEvent.KEYCODE_DPAD_DOWN: name = "Down"; break;
                case KeyEvent.KEYCODE_SPACE: name = "Space"; break;
                case KeyEvent.KEYCODE_ENTER: name = "Enter"; break;
                case KeyEvent.KEYCODE_DEL: name = "Backspace"; break;
                case KeyEvent.KEYCODE_ESCAPE: name = "Escape"; break;
                default:
                    int c = e.getUnicodeChar();
                    if (c == 0) return false;
                    name = new String(Character.toChars(c));
            }
            hear(changed(key), "key " + name);
            return true;
        }
    }

    class Board extends View {
        final Surface surface;
        final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        JSONArray ops = new JSONArray();

        Board(Context c, int number) {
            super(c);
            surface = new Surface(number);
            setFocusable(true);
            setFocusableInTouchMode(true);
        }

        void paint(JSONArray n, JSONArray ops) throws JSONException {
            surface.options(n);
            this.ops = ops;
            invalidate();
        }

        @Override
        protected void onSizeChanged(int w, int h, int oldW, int oldH) {
            surface.sized(w, h);
        }

        @Override
        protected void onMeasure(int w, int h) {
            setMeasuredDimension(getDefaultSize(dp(320), w), getDefaultSize(dp(200), h));
        }

        @Override
        public boolean onTouchEvent(MotionEvent e) {
            requestFocus();
            return surface.touch(e);
        }

        @Override
        public boolean onKeyDown(int code, KeyEvent e) {
            return surface.press(code, e) || super.onKeyDown(code, e);
        }

        void color(int c, double outline) {
            paint.setColor(0xFF000000 | c);
            paint.setStyle(outline != 0 ? Paint.Style.STROKE : Paint.Style.FILL);
            paint.setStrokeWidth((float) outline);
        }

        @Override
        protected void onDraw(Canvas g) {
            g.save();
            g.scale(density, density);
            try {
                for (int i = 0; i < ops.length(); i++) draw(g, ops.getJSONArray(i));
            } catch (JSONException e) {
                throw new RuntimeException(e);
            }
            g.restore();
        }

        void draw(Canvas g, JSONArray op) throws JSONException {
            switch (op.getString(0)) {
                case "C":
                    g.drawColor(0xFF000000 | op.getInt(1));
                    break;
                case "R": {
                    color(op.getInt(5), op.getDouble(6));
                    float x = (float) op.getDouble(1), y = (float) op.getDouble(2);
                    g.drawRect(x, y, x + (float) op.getDouble(3), y + (float) op.getDouble(4), paint);
                    break;
                }
                case "O":
                    color(op.getInt(4), op.getDouble(5));
                    g.drawCircle((float) op.getDouble(1), (float) op.getDouble(2), (float) op.getDouble(3), paint);
                    break;
                case "L":
                    color(op.getInt(5), op.getDouble(6));
                    paint.setStyle(Paint.Style.STROKE);
                    g.drawLine((float) op.getDouble(1), (float) op.getDouble(2), (float) op.getDouble(3), (float) op.getDouble(4), paint);
                    break;
                case "T": {
                    color(op.getInt(4), 0);
                    paint.setTextSize((float) op.getDouble(3));
                    // Kova's `y` is the top of the text.
                    g.drawText(op.getString(5), (float) op.getDouble(1), (float) op.getDouble(2) - paint.ascent(), paint);
                    break;
                }
                case "I": {
                    Bitmap b = picture(op.getString(5));
                    if (b == null) break;
                    float x = (float) op.getDouble(1), y = (float) op.getDouble(2);
                    double w = op.getDouble(3) != 0 ? op.getDouble(3) : b.getWidth();
                    double h = op.getDouble(4) != 0 ? op.getDouble(4) : b.getHeight();
                    g.drawBitmap(b, null, new RectF(x, y, x + (float) w, y + (float) h), paint);
                    break;
                }
                default:
                    break;
            }
        }
    }

    // ---- `ui.gpu`: the same steps as `gl_paint` in runtime/kova.rs, on OpenGL ES 3 ----

    class Scene extends GLSurfaceView implements GLSurfaceView.Renderer {
        final Surface surface;
        final Map<String, Integer> programs = new HashMap<>();
        volatile JSONArray ops = new JSONArray();
        int width, height;

        Scene(Context c, int number) {
            super(c);
            surface = new Surface(number);
            setEGLContextClientVersion(3);
            setPreserveEGLContextOnPause(true);
            setRenderer(this);
            setRenderMode(RENDERMODE_WHEN_DIRTY);
        }

        void paint(JSONArray n, JSONArray ops) throws JSONException {
            surface.options(n);
            this.ops = ops;
            requestRender();
        }

        @Override
        public boolean onTouchEvent(MotionEvent e) {
            return surface.touch(e);
        }

        @Override
        public void onSurfaceCreated(GL10 unused, EGLConfig config) {
            programs.clear();
            int[] one = new int[1];
            GLES30.glGenVertexArrays(1, one, 0);
            GLES30.glBindVertexArray(one[0]);
            GLES30.glGenBuffers(1, one, 0);
            GLES30.glBindBuffer(GLES30.GL_ARRAY_BUFFER, one[0]);
            GLES30.glEnable(GLES30.GL_DEPTH_TEST);
            GLES30.glDepthFunc(GLES30.GL_LEQUAL);
            GLES30.glEnable(GLES30.GL_BLEND);
            GLES30.glBlendFunc(GLES30.GL_SRC_ALPHA, GLES30.GL_ONE_MINUS_SRC_ALPHA);
        }

        @Override
        public void onSurfaceChanged(GL10 unused, int w, int h) {
            width = w;
            height = h;
            GLES30.glViewport(0, 0, w, h);
            ui.post(() -> surface.sized(w, h));
        }

        void wipe(int c) {
            GLES30.glClearColor((c >> 16 & 255) / 255f, (c >> 8 & 255) / 255f, (c & 255) / 255f, 1);
            GLES30.glClear(GLES30.GL_COLOR_BUFFER_BIT | GLES30.GL_DEPTH_BUFFER_BIT);
        }

        int compile(int kind, String source) {
            int shader = GLES30.glCreateShader(kind);
            // Written for OpenGL 3.3; only the first line differs.
            GLES30.glShaderSource(shader, source.replaceFirst("^\\s*#version.*", "#version 300 es\nprecision highp float;"));
            GLES30.glCompileShader(shader);
            int[] ok = new int[1];
            GLES30.glGetShaderiv(shader, GLES30.GL_COMPILE_STATUS, ok, 0);
            if (ok[0] == 0) hear(SHADER_FAILED, GLES30.glGetShaderInfoLog(shader));
            return shader;
        }

        @Override
        public void onDrawFrame(GL10 unused) {
            wipe(0);
            int program = 0;
            List<Integer> textures = new ArrayList<>();
            try {
                JSONArray ops = this.ops;
                for (int i = 0; i < ops.length(); i++) {
                    JSONArray op = ops.getJSONArray(i);
                    switch (op.getString(0)) {
                        case "c":
                            wipe(op.getInt(1));
                            break;
                        case "s": {
                            String key = op.getString(1) + "\0" + op.getString(2);
                            Integer made = programs.get(key);
                            if (made == null) {
                                made = GLES30.glCreateProgram();
                                GLES30.glAttachShader(made, compile(GLES30.GL_VERTEX_SHADER, op.getString(1)));
                                GLES30.glAttachShader(made, compile(GLES30.GL_FRAGMENT_SHADER, op.getString(2)));
                                GLES30.glLinkProgram(made);
                                programs.put(key, made);
                            }
                            program = made;
                            GLES30.glUseProgram(program);
                            break;
                        }
                        case "u": {
                            int at = GLES30.glGetUniformLocation(program, op.getString(1));
                            float[] v = floats(op.getJSONArray(2));
                            if (v.length == 16) GLES30.glUniformMatrix4fv(at, 1, false, v, 0);
                            else if (v.length == 1) GLES30.glUniform1fv(at, 1, v, 0);
                            else if (v.length == 2) GLES30.glUniform2fv(at, 1, v, 0);
                            else if (v.length == 3) GLES30.glUniform3fv(at, 1, v, 0);
                            else if (v.length == 4) GLES30.glUniform4fv(at, 1, v, 0);
                            break;
                        }
                        case "v": {
                            JSONArray sizes = op.getJSONArray(1);
                            int one = 0;
                            for (int k = 0; k < sizes.length(); k++) one += sizes.getInt(k);
                            if (one == 0) break;
                            float[] data = floats(op.getJSONArray(2));
                            FloatBuffer b = ByteBuffer.allocateDirect(data.length * 4).order(ByteOrder.nativeOrder()).asFloatBuffer();
                            b.put(data).position(0);
                            GLES30.glBufferData(GLES30.GL_ARRAY_BUFFER, data.length * 4, b, GLES30.GL_STREAM_DRAW);
                            int at = 0;
                            for (int k = 0; k < sizes.length(); k++) {
                                GLES30.glEnableVertexAttribArray(k);
                                GLES30.glVertexAttribPointer(k, sizes.getInt(k), GLES30.GL_FLOAT, false, one * 4, at * 4);
                                at += sizes.getInt(k);
                            }
                            GLES30.glDrawArrays(GLES30.GL_TRIANGLES, 0, data.length / one);
                            break;
                        }
                        case "x": {
                            int[] t = new int[1];
                            GLES30.glGenTextures(1, t, 0);
                            GLES30.glBindTexture(GLES30.GL_TEXTURE_2D, t[0]);
                            GLES30.glTexParameteri(GLES30.GL_TEXTURE_2D, GLES30.GL_TEXTURE_MAG_FILTER, GLES30.GL_LINEAR);
                            GLES30.glTexParameteri(GLES30.GL_TEXTURE_2D, GLES30.GL_TEXTURE_MIN_FILTER, GLES30.GL_LINEAR);
                            JSONArray pixels = op.getJSONArray(3);
                            ByteBuffer b = ByteBuffer.allocateDirect(pixels.length());
                            for (int k = 0; k < pixels.length(); k++) b.put((byte) pixels.getInt(k));
                            b.position(0);
                            GLES30.glTexImage2D(GLES30.GL_TEXTURE_2D, 0, GLES30.GL_RGBA8, op.getInt(1), op.getInt(2), 0, GLES30.GL_RGBA, GLES30.GL_UNSIGNED_BYTE, b);
                            textures.add(t[0]);
                            break;
                        }
                        default:
                            break;
                    }
                }
            } catch (JSONException e) {
                throw new RuntimeException(e);
            }
            for (int t : textures) GLES30.glDeleteTextures(1, new int[] {t}, 0);
        }
    }

    static float[] floats(JSONArray a) throws JSONException {
        float[] out = new float[a.length()];
        for (int i = 0; i < out.length; i++) out[i] = (float) a.getDouble(i);
        return out;
    }

    // ---- Dialogs, sound, the web ----

    void ask(String words) {
        String[] p = words.split("\n", 3);
        int kind = Integer.parseInt(p[0]);
        String title = p[1], text = p[2];
        switch (kind) {
            case 0: {
                Intent pick = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                pick.addCategory(Intent.CATEGORY_OPENABLE);
                pick.setType("*/*");
                startActivityForResult(pick, PICK_FILE);
                break;
            }
            case 1: {
                // A folder of the app's that the phone's Files app can open.
                File folder = getExternalFilesDir(null);
                String name = text.isEmpty() ? "kova.txt" : text;
                reply("Y" + new File(folder != null ? folder : getFilesDir(), name).getPath());
                break;
            }
            case 3:
                new AlertDialog.Builder(this).setTitle(title).setMessage(text).setCancelable(false)
                    .setPositiveButton(android.R.string.ok, (d, w) -> reply("N")).show();
                break;
            case 4:
                new AlertDialog.Builder(this).setTitle(title).setMessage(text).setCancelable(false)
                    .setPositiveButton(android.R.string.yes, (d, w) -> reply("Yyes"))
                    .setNegativeButton(android.R.string.no, (d, w) -> reply("N")).show();
                break;
            default:
                reply("N");
                break;
        }
    }

    @Override
    protected void onActivityResult(int code, int result, Intent data) {
        if (code != PICK_FILE) return;
        Uri uri = data == null ? null : data.getData();
        if (result != RESULT_OK || uri == null) {
            reply("N");
            return;
        }
        // A phone's documents have no path: the program gets a copy.
        String name = uri.getLastPathSegment();
        name = name == null ? "file" : name.substring(name.lastIndexOf('/') + 1).replace(':', '_');
        File copy = new File(getCacheDir(), name);
        try (InputStream in = getContentResolver().openInputStream(uri); OutputStream out = new FileOutputStream(copy)) {
            copyAll(in, out);
            reply("Y" + copy.getPath());
        } catch (IOException e) {
            reply("N");
        }
    }

    static void copyAll(InputStream in, OutputStream out) throws IOException {
        byte[] b = new byte[65536];
        for (int n; (n = in.read(b)) > 0; ) out.write(b, 0, n);
    }

    void play(byte[] wav) {
        try {
            File file = File.createTempFile("kova", ".wav", getCacheDir());
            try (OutputStream out = new FileOutputStream(file)) {
                out.write(wav);
            }
            MediaPlayer player = new MediaPlayer();
            player.setDataSource(file.getPath());
            player.setOnCompletionListener(p -> {
                p.release();
                file.delete();
                reply("Y");
            });
            player.setOnErrorListener((p, a, b) -> {
                reply("Y");
                return true;
            });
            player.prepare();
            player.start();
        } catch (IOException e) {
            reply("Y");
        }
    }

    void fetch(byte[] question) {
        int zero = 0;
        while (question[zero] != 0) zero++;
        try {
            JSONObject o = new JSONObject(new String(question, 0, zero, StandardCharsets.UTF_8));
            int body = question.length - zero - 1;
            String at = o.getString("url");
            boolean follow = o.getBoolean("follow");
            for (int hop = 0; ; hop++) {
                URL url = new URL(at);
                HttpURLConnection c = (HttpURLConnection) url.openConnection();
                int ms = (int) Math.min(o.getLong("ms"), Integer.MAX_VALUE);
                c.setConnectTimeout(ms);
                c.setReadTimeout(ms);
                c.setInstanceFollowRedirects(false);
                c.setRequestMethod(o.getString("method"));
                JSONArray headers = o.getJSONArray("headers");
                for (int i = 0; i < headers.length(); i++) {
                    c.setRequestProperty(headers.getJSONArray(i).getString(0), headers.getJSONArray(i).getString(1));
                }
                if (body > 0) {
                    c.setDoOutput(true);
                    c.setFixedLengthStreamingMode(body);
                    try (OutputStream out = c.getOutputStream()) {
                        out.write(question, zero + 1, body);
                    }
                }
                int status = c.getResponseCode();
                String next = c.getHeaderField("Location");
                if (follow && status >= 300 && status < 400 && next != null) {
                    if (hop >= 10) {
                        reply("r");
                        return;
                    }
                    at = new URL(url, next).toString();
                    continue;
                }
                ByteArrayOutputStream all = new ByteArrayOutputStream();
                all.write('+');
                StringBuilder head = new StringBuilder(status + "\n" + at + "\n");
                for (Map.Entry<String, List<String>> e : c.getHeaderFields().entrySet()) {
                    if (e.getKey() == null) continue;
                    for (String v : e.getValue()) head.append(e.getKey().toLowerCase(Locale.ROOT)).append(": ").append(v).append('\n');
                }
                head.append('\n');
                all.write(head.toString().getBytes(StandardCharsets.UTF_8));
                InputStream in = status >= 400 ? c.getErrorStream() : c.getInputStream();
                if (in != null) copyAll(in, all);
                reply(all.toByteArray());
                return;
            }
        } catch (javax.net.ssl.SSLException e) {
            reply("c");
        } catch (UnknownHostException e) {
            reply("h");
        } catch (SocketTimeoutException e) {
            reply("t");
        } catch (IOException | JSONException e) {
            reply("x");
        }
    }
}
