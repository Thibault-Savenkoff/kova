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
import android.app.DatePickerDialog;
import android.content.Context;
import android.content.Intent;
import android.content.res.ColorStateList;
import android.content.res.Configuration;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.Typeface;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
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
import android.widget.ProgressBar;
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
    // The Material 3 colors of the program's window (the phone's own when it has them).
    int cPrimary, cOnPrimary, cBackground, cOnSurface, cOnVariant, cCard, cPanel, cTonal, cOnTonal, cOutline;
    // The tab bar the view asks for (pinned at the bottom), the event of a back button, the buttons of the top bar.
    LinearLayout tabBar;
    long backEvent = -1;
    JSONArray barButtons = new JSONArray();
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
        boolean bold, marks;
        int lines;

        Style marked() {
            Style s = new Style();
            s.color = color;
            s.size = size;
            s.bold = bold;
            s.marks = true;
            s.lines = lines;
            return s;
        }

        Style withLines(int n) {
            Style s = marked();
            s.marks = marks;
            s.lines = n;
            return s;
        }

        Style over(JSONArray n) throws JSONException {
            Style s = new Style();
            s.marks = marks;
            s.lines = lines;
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
        palette(night);
        window = new FrameLayout(this);
        window.setBackgroundColor(cBackground);
        getWindow().setBackgroundDrawable(new ColorDrawable(cBackground));
        if (getActionBar() != null) getActionBar().setBackgroundDrawable(new ColorDrawable(cBackground));
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

    // ---- Material 3 ----

    /** Material 3's baseline colors; from Android 12 on, the ones the user's wallpaper gives. */
    void palette(boolean night) {
        if (night) {
            cPrimary = 0xFFD0BCFF; cOnPrimary = 0xFF381E72; cBackground = 0xFF141218; cOnSurface = 0xFFE6E0E9;
            cOnVariant = 0xFFCAC4D0; cCard = 0xFF211F26; cPanel = 0xFF1D1B20; cTonal = 0xFF4A4458;
            cOnTonal = 0xFFE8DEF8; cOutline = 0xFF938F99;
        } else {
            cPrimary = 0xFF6750A4; cOnPrimary = 0xFFFFFFFF; cBackground = 0xFFFEF7FF; cOnSurface = 0xFF1D1B20;
            cOnVariant = 0xFF49454F; cCard = 0xFFF3EDF7; cPanel = 0xFFF7F2FA; cTonal = 0xFFE8DEF8;
            cOnTonal = 0xFF1D192B; cOutline = 0xFF79747E;
        }
        if (Build.VERSION.SDK_INT >= 31) {
            try {
                cPrimary = getColor(night ? android.R.color.system_accent1_200 : android.R.color.system_accent1_600);
                cOnPrimary = getColor(night ? android.R.color.system_accent1_800 : android.R.color.system_accent1_0);
                cTonal = getColor(night ? android.R.color.system_accent2_700 : android.R.color.system_accent2_100);
                cOnTonal = getColor(night ? android.R.color.system_accent2_100 : android.R.color.system_accent2_900);
                cBackground = getColor(night ? android.R.color.system_neutral1_900 : android.R.color.system_neutral1_10);
                cPanel = getColor(night ? android.R.color.system_neutral2_900 : android.R.color.system_neutral2_10);
                cCard = getColor(night ? android.R.color.system_neutral2_800 : android.R.color.system_neutral2_50);
                cOnSurface = getColor(night ? android.R.color.system_neutral1_100 : android.R.color.system_neutral1_900);
                cOnVariant = getColor(night ? android.R.color.system_neutral2_200 : android.R.color.system_neutral2_700);
                cOutline = getColor(night ? android.R.color.system_neutral2_400 : android.R.color.system_neutral2_500);
            } catch (RuntimeException e) {
                // keep the baseline
            }
        }
    }

    /** A Kova color: 0xRRGGBB, or (past that) one of the system's: 1 panel, 2 page, 3 card. */
    int paint(long c) {
        if ((c >> 24) == 1) {
            switch ((int) (c & 255)) {
                case 1:
                    return cPanel;
                case 2:
                    return cBackground;
                default:
                    return cCard;
            }
        }
        return 0xFF000000 | (int) c;
    }

    GradientDrawable box(int fill, double radius, int stroke) {
        GradientDrawable g = new GradientDrawable();
        g.setColor(fill);
        g.setCornerRadius(dp(radius));
        if (stroke != 0) g.setStroke(dp(1), stroke);
        return g;
    }

    /** `fill` with a touch ripple, in the shape of its corners. */
    RippleDrawable ripple(int fill, double radius, int stroke) {
        int touch = (cOnSurface & 0x00FFFFFF) | 0x22000000;
        return new RippleDrawable(ColorStateList.valueOf(touch), box(fill, radius, stroke), box(Color.WHITE, radius, 0));
    }

    ColorStateList checkedTint() {
        return new ColorStateList(new int[][] {{android.R.attr.state_checked}, {}}, new int[] {cPrimary, cOutline});
    }

    /** A filled pill; a button with a color of its own is a text button. */
    void button(Button b, Style s) {
        b.setAllCaps(false);
        b.setStateListAnimator(null);
        b.setMinHeight(0);
        b.setMinimumHeight(0);
        b.setMinWidth(0);
        b.setMinimumWidth(0);
        b.setPadding(dp(24), dp(10), dp(24), dp(10));
        b.setTextSize(TypedValue.COMPLEX_UNIT_DIP, s.size != 0 ? (float) s.size : 14);
        b.setTypeface(null, Typeface.BOLD);
        boolean text = s.color != null;
        b.setTextColor(text ? paint(s.color) : cOnPrimary);
        b.setBackground(ripple(text ? Color.TRANSPARENT : cPrimary, 100, 0));
    }

    /** An outlined text field. */
    void field(EditText e) {
        e.setBackground(box(Color.TRANSPARENT, 12, cOutline));
        e.setPadding(dp(16), dp(12), dp(16), dp(12));
        e.setHintTextColor(cOnVariant);
    }

    /** The rows of a list: a card, a line for each row (title, subtitle, detail), a thin line between. */
    void rows(LinearLayout card, JSONArray n) throws JSONException {
        card.removeAllViews();
        JSONArray rows = n.getJSONArray(2);
        for (int i = 0; i < rows.length(); i++) {
            String[] parts = String.join(" ", strings(rows.getJSONArray(i))).split("\t");
            LinearLayout row = new LinearLayout(this);
            row.setOrientation(LinearLayout.HORIZONTAL);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(dp(16), dp(12), dp(16), dp(12));
            boolean chosen = i == n.getInt(3);
            row.setBackground(ripple(chosen ? cTonal : Color.TRANSPARENT, 0, 0));
            LinearLayout words = new LinearLayout(this);
            words.setOrientation(LinearLayout.VERTICAL);
            TextView title = new TextView(this);
            title.setText(parts[0]);
            title.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 16);
            title.setTextColor(cOnSurface);
            words.addView(title);
            if (parts.length > 1 && !parts[1].isEmpty()) {
                TextView sub = new TextView(this);
                sub.setText(parts[1]);
                sub.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 14);
                sub.setTextColor(cOnVariant);
                words.addView(sub);
            }
            row.addView(words, new LinearLayout.LayoutParams(0, -2, 1));
            if (parts.length > 2 && !parts[2].isEmpty()) {
                TextView more = new TextView(this);
                more.setText(parts[2]);
                more.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 12);
                more.setTextColor(cOnVariant);
                row.addView(more);
            }
            int number = i;
            long field = n.getLong(4);
            row.setOnClickListener(x -> hear(changed(field), String.valueOf(number)));
            if (n.length() > 6 && n.getBoolean(6)) {
                // A quick swipe to the left on a row asks to delete it.
                android.view.GestureDetector swipe = new android.view.GestureDetector(this, new android.view.GestureDetector.SimpleOnGestureListener() {
                    @Override
                    public boolean onDown(MotionEvent e) {
                        return true;
                    }

                    @Override
                    public boolean onFling(MotionEvent a, MotionEvent b, float vx, float vy) {
                        if (vx < -800 && Math.abs(vx) > Math.abs(vy)) {
                            hear(changed(field), "d" + number);
                            return true;
                        }
                        return false;
                    }
                });
                row.setOnTouchListener((x, e) -> {
                    swipe.onTouchEvent(e);
                    return false;
                });
            }
            card.addView(row, new LinearLayout.LayoutParams(-1, -2));
            if (i + 1 < rows.length()) {
                View line = new View(this);
                line.setBackgroundColor((cOutline & 0x00FFFFFF) | 0x33000000);
                LinearLayout.LayoutParams p = new LinearLayout.LayoutParams(-1, Math.max(1, dp(0.5)));
                p.leftMargin = dp(16);
                card.addView(line, p);
            }
        }
    }

    /** A view a column gives its whole width to. */
    boolean wide(JSONArray n) throws JSONException {
        switch (n.getString(0)) {
            case "b":
            case "q":
            case "p":
            case "s":
            case "d":
            case "G":
            case "C":
            case "I":
                return true;
            case "M":
                return wide(n.getJSONArray(2));
            case "t":
            case "r":
                return wide(n.getJSONArray(2));
            case "m":
                return wide(n.getJSONArray(1));
            case "n":
                return wide(n.getJSONArray(2));
            case "u":
                return wide(n.getJSONArray(3));
            case "S":
                return wide(n.getJSONArray(4));
            case "E":
                return wide(n.getJSONArray(3));
            case "J":
                return wide(n.getJSONArray(2));
            default:
                return false;
        }
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
                        consoleBox.post(() -> consoleBox.scrollTo(0, consoleColumn.getHeight()));
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
                        getSystemService(android.view.inputmethod.InputMethodManager.class).showSoftInput(consoleLine, 0);
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
        backEvent = -1;
        JSONArray before = barButtons;
        barButtons = new JSONArray();
        if (seen.equals(shape)) {
            update(root, view, s);
        } else {
            shape = seen;
            tabBar = null;
            root = build(view, s);
            window.removeAllViews();
            int gap = dp(8);
            LinearLayout screen = new LinearLayout(this);
            screen.setOrientation(LinearLayout.VERTICAL);
            screen.setBackgroundColor(cBackground);
            screen.setDefaultFocusHighlightEnabled(false);
            if (fills(view, 1)) {
                root.setPadding(gap, gap, gap, gap);
                screen.addView(root, new LinearLayout.LayoutParams(-1, 0, 1));
            } else {
                ScrollView scroll = new ScrollView(this);
                scroll.setPadding(gap, gap, gap, gap);
                scroll.setClipToPadding(false);
                // A scroll view taking the focus would be covered by the system's gray focus highlight.
                scroll.setDefaultFocusHighlightEnabled(false);
                scroll.addView(root, new ViewGroup.LayoutParams(-1, -2));
                screen.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));
            }
            if (tabBar != null) screen.addView(tabBar, new LinearLayout.LayoutParams(-1, -2));
            window.addView(screen, new FrameLayout.LayoutParams(-1, -1));
            showing = true;
        }
        if (!before.toString().equals(barButtons.toString()) || (backEvent >= 0) != (getActionBar() != null && (getActionBar().getDisplayOptions() & android.app.ActionBar.DISPLAY_HOME_AS_UP) != 0)) {
            if (getActionBar() != null) getActionBar().setDisplayHomeAsUpEnabled(backEvent >= 0);
            invalidateOptionsMenu();
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
            case "J":
            case "t":
            case "r":
                clocksOf(n.getJSONArray(2), found);
                break;
            case "m":
                clocksOf(n.getJSONArray(1), found);
                break;
            case "n":
                clocksOf(n.getJSONArray(2), found);
                break;
            case "b":
            case "s":
                clocksOf(n.getJSONArray(3), found);
                break;
            case "u":
                clocksOf(n.getJSONArray(3), found);
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
            case "J":
            case "t":
            case "r":
                return fills(n.getJSONArray(2), side);
            case "m":
                return fills(n.getJSONArray(1), side);
            case "n":
                return fills(n.getJSONArray(2), side);
            case "b":
            case "s":
            case "u":
                return fills(n.getJSONArray(3), side);
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
        v.setTextColor(s.color != null ? paint(s.color) : cOnSurface);
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
                button(b, s);
                b.setOnClickListener(x -> hear((long) x.getTag(), ""));
                v = b;
                break;
            }
            case "I": {
                EditText e = new EditText(this);
                e.setSingleLine(true);
                e.setInputType(InputType.TYPE_CLASS_TEXT);
                field(e);
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
                c.setButtonTintList(checkedTint());
                c.setTextColor(cOnSurface);
                c.setOnCheckedChangeListener((x, on) -> {
                    if (x.getTag(R_APPLYING) == null) hear(changed((long) x.getTag()), on ? "on" : "off");
                });
                v = c;
                break;
            }
            case "W": {
                SeekBar bar = new SeekBar(this);
                bar.setMinimumWidth(dp(200));
                bar.setProgressTintList(ColorStateList.valueOf(cPrimary));
                bar.setThumbTintList(ColorStateList.valueOf(cPrimary));
                bar.setProgressBackgroundTintList(ColorStateList.valueOf(cTonal));
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
                    boolean w = fills(part, 0) || (!row && wide(part)), h = fills(part, 1);
                    LinearLayout.LayoutParams p = row
                        ? new LinearLayout.LayoutParams(w ? 0 : -2, h ? -1 : -2, w ? 1 : 0)
                        : new LinearLayout.LayoutParams(w ? -1 : -2, h ? 0 : -2, h ? 1 : 0);
                    if (i > 0) {
                        if (row) p.leftMargin = dp(8);
                        else p.topMargin = dp(parts.getJSONArray(i - 1).getString(0).equals("b") ? 16 : 8);
                    }
                    l.addView(build(part, s), p);
                }
                v = l;
                break;
            }
            case "M": {
                FrameLayout box = new FrameLayout(this);
                JSONArray inner = n.getJSONArray(2);
                box.addView(build(inner, s), new FrameLayout.LayoutParams(fills(inner, 0) || wide(inner) ? -1 : -2, fills(inner, 1) ? -1 : -2));
                v = box;
                break;
            }
            case "m":
                return build(n.getJSONArray(1), s.marked());
            case "n":
                return build(n.getJSONArray(2), s.withLines(n.getInt(1)));
            case "S":
                return build(n.getJSONArray(4), s.over(n));
            case "E":
                return build(n.getJSONArray(3), s);
            case "J":
                v = build(n.getJSONArray(2), s);
                context(v, n.getJSONArray(1));
                return v;
            case "b": {
                FrameLayout card = new FrameLayout(this);
                JSONArray inner = n.getJSONArray(3);
                card.addView(build(inner, s), new FrameLayout.LayoutParams(fills(inner, 0) || wide(inner) ? -1 : -2, fills(inner, 1) ? -1 : -2));
                v = card;
                break;
            }
            case "t":
                setTitle(n.getString(1));
                return build(n.getJSONArray(2), s);
            case "u":
                backEvent = n.getLong(1);
                return build(n.getJSONArray(3), s);
            case "r":
                barButtons = n.getJSONArray(1);
                return build(n.getJSONArray(2), s);
            case "s": {
                LinearLayout l = new LinearLayout(this);
                l.setOrientation(LinearLayout.VERTICAL);
                l.addView(searchField(n.getLong(2)), new LinearLayout.LayoutParams(-1, -2));
                JSONArray inner = n.getJSONArray(3);
                LinearLayout.LayoutParams p = new LinearLayout.LayoutParams(wide(inner) ? -1 : -2, fills(inner, 1) ? 0 : -2, fills(inner, 1) ? 1 : 0);
                p.topMargin = dp(8);
                l.addView(build(inner, s), p);
                v = l;
                break;
            }
            case "q":
                v = searchField(n.getLong(2));
                break;
            case "H": {
                tabBar = new LinearLayout(this);
                tabBar.setOrientation(LinearLayout.HORIZONTAL);
                tabBar.setBackgroundColor(cCard);
                tabBar.setPadding(dp(8), dp(12), dp(8), dp(12));
                updateTabs(n);
                View hole = new View(this);
                hole.setVisibility(View.GONE);
                return hole;
            }
            case "d": {
                EditText e = new EditText(this);
                e.setFocusable(false);
                e.setSingleLine(true);
                field(e);
                e.setOnClickListener(x -> {
                    java.util.Calendar c = java.util.Calendar.getInstance();
                    String[] d = e.getText().toString().split("-");
                    try {
                        c.set(Integer.parseInt(d[0]), Integer.parseInt(d[1]) - 1, Integer.parseInt(d[2]));
                    } catch (RuntimeException ignored) {
                        // today
                    }
                    new DatePickerDialog(this, (picker, y, m, day) -> hear(changed((long) e.getTag()), String.format(Locale.ROOT, "%04d-%02d-%02d", y, m + 1, day)), c.get(java.util.Calendar.YEAR), c.get(java.util.Calendar.MONTH), c.get(java.util.Calendar.DAY_OF_MONTH)).show();
                });
                v = e;
                break;
            }
            case "p": {
                ProgressBar bar = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
                bar.setProgressTintList(ColorStateList.valueOf(cPrimary));
                bar.setProgressBackgroundTintList(ColorStateList.valueOf(cTonal));
                v = bar;
                break;
            }
            case "e": {
                LinearLayout l = new LinearLayout(this);
                l.setOrientation(LinearLayout.HORIZONTAL);
                l.setGravity(Gravity.CENTER_VERTICAL);
                for (int k = 0; k < 3; k++) {
                    TextView t = new TextView(this);
                    t.setGravity(Gravity.CENTER);
                    t.setTextSize(TypedValue.COMPLEX_UNIT_DIP, k == 1 ? 18 : 22);
                    t.setTextColor(k == 1 ? cOnSurface : cOnTonal);
                    if (k != 1) t.setBackground(ripple(cTonal, 100, 0));
                    t.setText(k == 0 ? "\u2212" : k == 2 ? "+" : "");
                    int step = k == 0 ? -1 : 1;
                    if (k != 1) {
                        t.setOnClickListener(x -> {
                            long[] now = (long[]) l.getTag(R_APPLYING);
                            long next = Math.max(now[1], Math.min(now[2], now[0] + step));
                            hear(changed(now[3]), String.valueOf(next));
                        });
                    }
                    l.addView(t, new LinearLayout.LayoutParams(k == 1 ? dp(48) : dp(40), dp(40)));
                }
                v = l;
                break;
            }
            case "z": {
                ProgressBar spin = new ProgressBar(this);
                spin.setIndeterminateTintList(ColorStateList.valueOf(cPrimary));
                v = spin;
                break;
            }
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
    static final int R_MANY = 0x7F010002;

    static List<String> strings(JSONArray a) throws JSONException {
        List<String> out = new ArrayList<>();
        for (int i = 0; i < a.length(); i++) out.add(a.getString(i));
        return out;
    }

    View table(JSONArray n) throws JSONException {
        if (n.getJSONArray(1).length() == 0) {
            // A list: one card, a row for each item.
            LinearLayout card = new LinearLayout(this);
            card.setOrientation(LinearLayout.VERTICAL);
            card.setBackground(box(cCard, 16, 0));
            card.setClipToOutline(true);
            return card;
        }
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
                if (s.marks) {
                    t.setText(marked(n.getString(1)));
                    t.setMovementMethod(android.text.method.LinkMovementMethod.getInstance());
                } else if (!t.getText().toString().equals(n.getString(1))) {
                    t.setText(n.getString(1));
                }
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
                boolean many = s.lines > 1;
                if (many != (e.getTag(R_MANY) != null)) {
                    e.setTag(R_MANY, many ? true : null);
                    e.setSingleLine(!many);
                    e.setInputType(many ? InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES : InputType.TYPE_CLASS_TEXT);
                    e.setGravity(many ? Gravity.TOP | Gravity.START : Gravity.CENTER_VERTICAL | Gravity.START);
                }
                if (many) e.setMinLines(s.lines);
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
                if (v instanceof LinearLayout) {
                    ((LinearLayout) v).setTag(n.getLong(4));
                    rows((LinearLayout) v, n);
                    break;
                }
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
            case "m":
                update(v, n.getJSONArray(1), s.marked());
                break;
            case "n":
                update(v, n.getJSONArray(2), s.withLines(n.getInt(1)));
                break;
            case "S":
                update(v, n.getJSONArray(4), s.over(n));
                break;
            case "E":
                update(v, n.getJSONArray(3), s);
                break;
            case "b": {
                v.setBackground(box(paint(n.getLong(1)), n.getInt(2) * 0.6, 0));
                ((FrameLayout) v).setClipToOutline(true);
                update(((FrameLayout) v).getChildAt(0), n.getJSONArray(3), s);
                break;
            }
            case "t":
                setTitle(n.getString(1));
                update(v, n.getJSONArray(2), s);
                break;
            case "u":
                backEvent = n.getLong(1);
                update(v, n.getJSONArray(3), s);
                break;
            case "r":
                barButtons = n.getJSONArray(1);
                update(v, n.getJSONArray(2), s);
                break;
            case "s": {
                LinearLayout l = (LinearLayout) v;
                searching((EditText) l.getChildAt(0), n.getString(1), n.getLong(2));
                update(l.getChildAt(1), n.getJSONArray(3), s);
                break;
            }
            case "q":
                searching((EditText) v, n.getString(1), n.getLong(2));
                break;
            case "H":
                updateTabs(n);
                break;
            case "d": {
                EditText e = (EditText) v;
                e.setTag(n.getLong(2));
                if (!e.getText().toString().equals(n.getString(1))) e.setText(n.getString(1));
                text(e, s);
                break;
            }
            case "p": {
                ProgressBar bar = (ProgressBar) v;
                bar.setMax(Math.max(1, n.getInt(2)));
                bar.setProgress(Math.max(0, Math.min(n.getInt(2), n.getInt(1))));
                break;
            }
            case "e": {
                LinearLayout l = (LinearLayout) v;
                l.setTag(R_APPLYING, new long[] {n.getLong(1), n.getLong(2), n.getLong(3), n.getLong(4)});
                ((TextView) l.getChildAt(1)).setText(String.valueOf(n.getLong(1)));
                break;
            }
            case "z":
                break;
            case "J":
                update(v, n.getJSONArray(2), s);
                context(v, n.getJSONArray(1));
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

    /** `**bold**`, `*stressed*`, `code` and `[label](address)` in a text, as what they mean. */
    CharSequence marked(String text) {
        android.text.SpannableStringBuilder out = new android.text.SpannableStringBuilder();
        int bold = -1, stress = -1, code = -1, flag = android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE;
        for (int i = 0; i < text.length();) {
            char c = text.charAt(i);
            char next = i + 1 < text.length() ? text.charAt(i + 1) : 0;
            if (c == '`') {
                if (code >= 0 && out.length() > code) {
                    out.setSpan(new android.text.style.TypefaceSpan("monospace"), code, out.length(), flag);
                    out.setSpan(new android.text.style.BackgroundColorSpan(cTonal), code, out.length(), flag);
                    code = -1;
                } else {
                    code = out.length();
                }
                i++;
            } else if (code >= 0) {
                out.append(c);
                i++;
            } else if (c == '*' && next == '*') {
                if (bold >= 0) {
                    out.setSpan(new android.text.style.StyleSpan(Typeface.BOLD), bold, out.length(), flag);
                    bold = -1;
                } else {
                    bold = out.length();
                }
                i += 2;
            } else if (c == '*') {
                if (stress >= 0) {
                    out.setSpan(new android.text.style.StyleSpan(Typeface.ITALIC), stress, out.length(), flag);
                    stress = -1;
                } else {
                    stress = out.length();
                }
                i++;
            } else if (c == '[') {
                int m = text.indexOf("](", i + 1);
                int e = m < 0 ? -1 : text.indexOf(')', m + 2);
                if (m >= 0 && e >= 0) {
                    String address = text.substring(m + 2, e);
                    int from = out.length();
                    out.append(text.substring(i + 1, m));
                    out.setSpan(new android.text.style.ClickableSpan() {
                        @Override
                        public void onClick(View v) {
                            try {
                                startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(address)));
                            } catch (RuntimeException ignored) {
                                // no program for it
                            }
                        }

                        @Override
                        public void updateDrawState(android.text.TextPaint p) {
                            p.setColor(cPrimary);
                            p.setUnderlineText(true);
                        }
                    }, from, out.length(), flag);
                    i = e + 1;
                } else {
                    out.append(c);
                    i++;
                }
            } else {
                out.append(c);
                i++;
            }
        }
        return out;
    }

    /** A rounded search field with a magnifying glass; each keystroke is an event, like a text field. */
    EditText searchField(long field) {
        EditText e = new EditText(this);
        e.setSingleLine(true);
        e.setInputType(InputType.TYPE_CLASS_TEXT);
        e.setHint("Search");
        android.graphics.drawable.Drawable glass = getDrawable(android.R.drawable.ic_menu_search).mutate();
        glass.setTint(cOnVariant);
        glass.setBounds(0, 0, dp(20), dp(20));
        e.setCompoundDrawablesRelative(glass, null, null, null);
        e.setCompoundDrawablePadding(dp(12));
        e.setBackground(box(cCard, 100, 0));
        e.setPadding(dp(16), dp(10), dp(16), dp(10));
        e.setHintTextColor(cOnVariant);
        e.setTextColor(cOnSurface);
        e.setTag(field);
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
        return e;
    }

    void searching(EditText e, String value, long field) {
        e.setTag(field);
        if (!e.getText().toString().equals(value)) {
            e.setTag(R_APPLYING, true);
            e.setText(value);
            e.setSelection(e.length());
            e.setTag(R_APPLYING, null);
        }
    }

    // ---- Icons: Material's outlines, from the name of an iPhone symbol ----

    static String iconPath(String name) {
        if (name.contains("note") || name.contains("doc")) return "M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z";
        if (name.contains("chart")) return "M5 9.2h3V19H5zM10.6 5h2.8v14h-2.8zm5.6 8H19v6h-2.8z";
        if (name.contains("info")) return "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z";
        if (name.contains("gear") || name.contains("setting")) return "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z";
        if (name.contains("house") || name.contains("home")) return "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z";
        if (name.contains("person") || name.contains("user")) return "M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z";
        if (name.contains("star")) return "M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z";
        if (name.contains("heart")) return "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z";
        if (name.contains("magnif") || name.contains("search")) return "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z";
        if (name.contains("plus")) return "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z";
        if (name.contains("list")) return "M3 13h2v-2H3v2zm0 4h2v-2H3v2zm0-8h2V7H3v2zm4 4h14v-2H7v2zm0 4h14v-2H7v2zM7 7v2h14V7H7z";
        if (name.contains("calendar")) return "M19 3h-1V1h-2v2H8V1H6v2H5c-1.11 0-1.99.9-1.99 2L3 19c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H5V8h14v11z";
        if (name.contains("folder")) return "M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z";
        if (name.contains("bell")) return "M12 22c1.1 0 2-.9 2-2h-4c0 1.1.89 2 2 2zm6-6v-5c0-3.07-1.64-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.63 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z";
        return "M12 6a6 6 0 1 0 0 12 6 6 0 0 0 0-12z";
    }

    /** A path in SVG's notation (M L H V C S Z, upper and lower case), on a 24 by 24 grid. */
    static android.graphics.Path svg(String d) {
        android.graphics.Path p = new android.graphics.Path();
        java.util.regex.Matcher m = java.util.regex.Pattern.compile("([a-zA-Z])|(-?\\d*\\.?\\d+(?:e-?\\d+)?)").matcher(d);
        List<String> t = new ArrayList<>();
        while (m.find()) t.add(m.group());
        float x = 0, y = 0, sx = 0, sy = 0, cx = 0, cy = 0;
        char cmd = 'M';
        int i = 0;
        while (i < t.size()) {
            if (Character.isLetter(t.get(i).charAt(0))) cmd = t.get(i++).charAt(0);
            boolean rel = Character.isLowerCase(cmd);
            float ox = rel ? x : 0, oy = rel ? y : 0;
            switch (Character.toUpperCase(cmd)) {
                case 'M':
                    x = ox + Float.parseFloat(t.get(i++));
                    y = oy + Float.parseFloat(t.get(i++));
                    p.moveTo(x, y);
                    sx = x;
                    sy = y;
                    cmd = rel ? 'l' : 'L';
                    cx = x;
                    cy = y;
                    break;
                case 'L':
                    x = ox + Float.parseFloat(t.get(i++));
                    y = oy + Float.parseFloat(t.get(i++));
                    p.lineTo(x, y);
                    cx = x;
                    cy = y;
                    break;
                case 'H':
                    x = ox + Float.parseFloat(t.get(i++));
                    p.lineTo(x, y);
                    cx = x;
                    cy = y;
                    break;
                case 'V':
                    y = oy + Float.parseFloat(t.get(i++));
                    p.lineTo(x, y);
                    cx = x;
                    cy = y;
                    break;
                case 'C': {
                    float x1 = ox + Float.parseFloat(t.get(i++)), y1 = oy + Float.parseFloat(t.get(i++));
                    float x2 = ox + Float.parseFloat(t.get(i++)), y2 = oy + Float.parseFloat(t.get(i++));
                    x = ox + Float.parseFloat(t.get(i++));
                    y = oy + Float.parseFloat(t.get(i++));
                    p.cubicTo(x1, y1, x2, y2, x, y);
                    cx = x2;
                    cy = y2;
                    break;
                }
                case 'S': {
                    float x1 = 2 * x - cx, y1 = 2 * y - cy;
                    float x2 = ox + Float.parseFloat(t.get(i++)), y2 = oy + Float.parseFloat(t.get(i++));
                    x = ox + Float.parseFloat(t.get(i++));
                    y = oy + Float.parseFloat(t.get(i++));
                    p.cubicTo(x1, y1, x2, y2, x, y);
                    cx = x2;
                    cy = y2;
                    break;
                }
                case 'A': {
                    // An arc: only the circles of the plain icon, drawn by their ends.
                    float r = Float.parseFloat(t.get(i++));
                    i += 4;
                    float ex = ox + Float.parseFloat(t.get(i++)), ey = oy + Float.parseFloat(t.get(i++));
                    p.addCircle((x + ex) / 2, (y + ey) / 2, r, android.graphics.Path.Direction.CW);
                    x = ex;
                    y = ey;
                    break;
                }
                case 'Z':
                    p.close();
                    x = sx;
                    y = sy;
                    break;
                default:
                    return p;
            }
        }
        return p;
    }

    /** The icon a symbol name stands for, `size` dp wide, in `color`. */
    android.graphics.drawable.Drawable icon(String name, int color, int size) {
        android.graphics.Path path = svg(iconPath(name));
        Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        paint.setColor(color);
        android.graphics.drawable.Drawable d = new android.graphics.drawable.Drawable() {
            @Override
            public void draw(Canvas g) {
                android.graphics.Rect b = getBounds();
                g.save();
                g.translate(b.left, b.top);
                g.scale(b.width() / 24f, b.height() / 24f);
                g.drawPath(path, paint);
                g.restore();
            }

            @Override
            public int getIntrinsicWidth() {
                return dp(size);
            }

            @Override
            public int getIntrinsicHeight() {
                return dp(size);
            }

            @Override
            public void setAlpha(int a) {}

            @Override
            public void setColorFilter(android.graphics.ColorFilter f) {}

            @Override
            public int getOpacity() {
                return android.graphics.PixelFormat.TRANSLUCENT;
            }
        };
        d.setBounds(0, 0, dp(size), dp(size));
        return d;
    }

    /** The bottom bar: a label for each tab, a pill behind the one shown. A tab shown is a text, the others buttons. */
    void updateTabs(JSONArray n) throws JSONException {
        tabBar.removeAllViews();
        JSONArray parts = n.getJSONArray(2), icons = n.getJSONArray(1);
        for (int i = 0; i < parts.length(); i++) {
            JSONArray part = parts.getJSONArray(i);
            boolean shown = part.getString(0).equals("T");
            String symbol = i < icons.length() ? icons.getString(i) : "";
            LinearLayout tab = new LinearLayout(this);
            tab.setOrientation(LinearLayout.VERTICAL);
            tab.setGravity(Gravity.CENTER_HORIZONTAL);
            TextView t = new TextView(this);
            t.setText(part.getString(1));
            t.setGravity(Gravity.CENTER);
            t.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 12);
            t.setTypeface(null, shown ? Typeface.BOLD : Typeface.NORMAL);
            t.setTextColor(shown ? cOnSurface : cOnVariant);
            if (symbol.isEmpty()) {
                t.setPadding(dp(12), dp(8), dp(12), dp(8));
                t.setBackground(ripple(shown ? cTonal : Color.TRANSPARENT, 100, 0));
                tab.addView(t);
            } else {
                // A pill behind the icon, the name under it.
                ImageView pill = new ImageView(this);
                pill.setImageDrawable(icon(symbol, shown ? cOnTonal : cOnVariant, 24));
                pill.setScaleType(ImageView.ScaleType.CENTER);
                pill.setBackground(ripple(shown ? cTonal : Color.TRANSPARENT, 100, 0));
                tab.addView(pill, new LinearLayout.LayoutParams(dp(64), dp(32)));
                t.setPadding(0, dp(4), 0, 0);
                tab.addView(t);
            }
            if (!shown) {
                long event = part.getLong(2);
                tab.setOnClickListener(x -> hear(event, ""));
            }
            LinearLayout.LayoutParams p = new LinearLayout.LayoutParams(0, -2, 1);
            tabBar.addView(tab, p);
        }
    }

    /** A long press on `v` opens a menu of `items` ([label, event number]), the phone's right click. */
    void context(View v, JSONArray items) {
        v.setOnLongClickListener(x -> {
            android.widget.PopupMenu menu = new android.widget.PopupMenu(this, x);
            try {
                for (int i = 0; i < items.length(); i++) menu.getMenu().add(0, i, 0, items.getJSONArray(i).getString(0));
                menu.setOnMenuItemClickListener(item -> {
                    try {
                        hear(items.getJSONArray(item.getItemId()).getLong(1), "");
                    } catch (JSONException e) {
                        return false;
                    }
                    return true;
                });
            } catch (JSONException e) {
                return false;
            }
            menu.show();
            return true;
        });
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
            // The buttons the view asks for sit in the bar itself.
            for (int i = 0; i < barButtons.length(); i++) {
                JSONArray b = barButtons.getJSONArray(i);
                MenuItem item = menu.add(0, id, 0, b.getString(0));
                item.setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS);
                menuAnswers.put(id++, b.getLong(1));
            }
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
    public void onBackPressed() {
        if (showing && backEvent >= 0) hear(backEvent, "");
        else super.onBackPressed();
    }

    @Override
    public boolean onOptionsItemSelected(MenuItem item) {
        if (item.getItemId() == android.R.id.home && backEvent >= 0) {
            hear(backEvent, "");
            return true;
        }
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
