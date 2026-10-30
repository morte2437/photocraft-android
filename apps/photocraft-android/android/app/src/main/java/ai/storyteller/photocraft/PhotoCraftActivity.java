package ai.storyteller.photocraft;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.Intent;
import android.content.SharedPreferences;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.os.Trace;
import android.provider.OpenableColumns;
import android.util.Log;
import android.view.MotionEvent;
import android.webkit.MimeTypeMap;
import android.widget.Toast;

import androidx.core.view.ViewCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidgamesdk.GameActivity;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

/**
 * PhotoCraft's Activity: GameActivity runs the Rust app (libphotocraft_android.so); this class
 * adds what winit doesn't offer on Android.
 *
 * <ul>
 *   <li><b>Pen</b>: winit forwards pressure only, so every MotionEvent's stylus axes (pressure,
 *       tilt, orientation, tip or eraser end) go to Rust through {@link #nativePen} before the
 *       event continues to GameActivity.</li>
 *   <li><b>Documents</b>: Storage Access Framework pickers, driven by small JSON files in
 *       {@code files/bridge} (see {@code src/bridge.rs} for the protocol). Picked documents are
 *       copied in; files the app writes are copied back to the document they stand for.</li>
 *   <li><b>Open with / Share</b>: sent documents are copied to {@code bridge/incoming}.</li>
 * </ul>
 */
public class PhotoCraftActivity extends GameActivity {
    private static final String TAG = "PhotoCraft";
    private static final int REQ_OPEN = 1001;
    private static final int REQ_SAVE = 1002;
    private static final long POLL_MS = 150;

    static {
        // GameActivity loads the library too (android.app.lib_name); loading it here first makes
        // nativePen resolvable from the first touch.
        System.loadLibrary("photocraft_android");
    }

    /** One pointer of a MotionEvent (photocraft-tablet's android.rs). */
    private static native void nativePen(int action, int tool, int buttons, float pressure, float tilt, float orientation, float x, float y, int ageMs,
            float size, float touchMajor);

    /** A pointer went down: its id, tool type and buttons, so Rust tells fingers from pens and a
     * right-click from a click (touch gestures,
     * palm rejection; photocraft-tablet's android.rs). */
    private static native void nativePointer(int id, int tool, int buttons);

    /** The on-screen keyboard's height in pixels (0: hidden), for photocraft-tablet's android.rs. */
    private static native void nativeImeHeight(int px);

    /** Where the front camera's cutouts are (up to two, window pixels; n says how many), so the
     * menu and options bars split around them (photocraft-tablet's android.rs). */
    private static native void nativeCutout(int n, int l0, int t0, int r0, int b0, int l1, int t1, int r1, int b1);
    /** The screen's rounded corners: radius and centre of the top-left, top-right, bottom-right and bottom-left (0 radius: square). */
    private static native void nativeCorners(int r0, int x0, int y0, int r1, int x1, int y1, int r2, int x2, int y2, int r3, int x3, int y3);

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final ExecutorService io = Executors.newSingleThreadExecutor();
    private File bridge;
    private Updater updater;
    /** The picker request being answered (-1: none), kept across Activity recreation. */
    private long pendingId = -1;
    private String pendingName = "";
    private int incomingSeq = 0;

    private final Runnable poll = new Runnable() {
        @Override
        public void run() {
            try {
                checkRequest();
                checkSync();
                checkQuit();
                checkUpdateRequest();
                checkClipboard();
            } catch (Exception e) {
                Log.e(TAG, "bridge", e);
            }
            handler.postDelayed(this, POLL_MS);
        }
    };

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        bridge = new File(getFilesDir(), "bridge");
        updater = new Updater(this);
        if (state == null && updater.checkAtStartup()) {
            // A moment after launch, so the first frames aren't competing with it.
            handler.postDelayed(() -> updater.check(false), 4000);
        }
        if (state != null) {
            pendingId = state.getLong("pendingId", -1);
            pendingName = state.getString("pendingName", "");
        }
        hideSystemBars();
        // Draw under a front camera's cutout on every Android version, as Android 15 does for
        // apps targeting it; the menu and options bars split around the camera instead (#21).
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.P) {
            android.view.WindowManager.LayoutParams lp = getWindow().getAttributes();
            lp.layoutInDisplayCutoutMode = android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R
                    ? android.view.WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                    : android.view.WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            getWindow().setAttributes(lp);
        }
        handleSentDocument(getIntent());
        // The on-screen keyboard's height, so the app lays out above it instead of under it.
        android.view.View decor = getWindow().getDecorView();
        decor.getViewTreeObserver().addOnGlobalLayoutListener(() -> {
            WindowInsetsCompat insets = ViewCompat.getRootWindowInsets(decor);
            int ime = insets == null ? 0 : insets.getInsets(WindowInsetsCompat.Type.ime()).bottom;
            // A front camera's cutout (a punch hole or notch): the bars split around it.
            androidx.core.view.DisplayCutoutCompat dc = insets == null ? null : insets.getDisplayCutout();
            List<android.graphics.Rect> cut = dc == null ? new ArrayList<>() : dc.getBoundingRects();
            if (!cut.equals(lastCutout)) {
                lastCutout = cut;
                android.graphics.Rect a = cut.size() > 0 ? cut.get(0) : new android.graphics.Rect();
                android.graphics.Rect b = cut.size() > 1 ? cut.get(1) : new android.graphics.Rect();
                try {
                    nativeCutout(Math.min(cut.size(), 2), a.left, a.top, a.right, a.bottom, b.left, b.top, b.right, b.bottom);
                } catch (UnsatisfiedLinkError err) {
                    lastCutout = null;
                }
            }
            // The screen's rounded corners (Android 12+): controls in them move clear of the curve.
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.S) {
                android.view.WindowInsets raw = decor.getRootWindowInsets();
                int[] c = new int[12];
                int[] pos = {android.view.RoundedCorner.POSITION_TOP_LEFT, android.view.RoundedCorner.POSITION_TOP_RIGHT,
                        android.view.RoundedCorner.POSITION_BOTTOM_RIGHT, android.view.RoundedCorner.POSITION_BOTTOM_LEFT};
                for (int i = 0; raw != null && i < 4; i++) {
                    android.view.RoundedCorner rc = raw.getRoundedCorner(pos[i]);
                    if (rc != null) {
                        c[3 * i] = rc.getRadius();
                        c[3 * i + 1] = rc.getCenter().x;
                        c[3 * i + 2] = rc.getCenter().y;
                    }
                }
                if (!java.util.Arrays.equals(c, lastCorners)) {
                    lastCorners = c;
                    try {
                        nativeCorners(c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7], c[8], c[9], c[10], c[11]);
                    } catch (UnsatisfiedLinkError err) {
                        lastCorners = null;
                    }
                }
            }
            if (ime != lastImeHeight) {
                lastImeHeight = ime;
                try {
                    nativeImeHeight(ime);
                } catch (UnsatisfiedLinkError err) {
                    // The library isn't loaded yet: the next layout tells it.
                    lastImeHeight = -1;
                }
            }
        });
    }

    private int lastImeHeight = -1;
    private List<android.graphics.Rect> lastCutout = null;
    private int[] lastCorners = null;

    @Override
    protected void onResume() {
        super.onResume();
        hideSystemBars();
        handler.removeCallbacks(poll);
        handler.post(poll);
        updater.resumePendingInstall();
    }

    @Override
    protected void onPause() {
        handler.removeCallbacks(poll);
        super.onPause();
    }

    /**
     * Back sends PhotoCraft to the background rather than finishing it: finishing would leave
     * without the unsaved-work prompt (File › Exit has it).
     */
    @Override
    public void onBackPressed() {
        moveTaskToBack(true);
    }

    /** The keyboard was put away: tell the Shortcut Pad (its keyboard key goes dark). */
    @Override
    public void onSoftwareKeyboardVisibilityChanged(boolean visible) {
        super.onSoftwareKeyboardVisibilityChanged(visible);
        if (!visible && bridge != null) {
            try {
                new File(bridge, "keyboard-hidden").createNewFile();
            } catch (java.io.IOException e) {
                Log.w(TAG, "keyboard-hidden", e);
            }
        }
    }

    /**
     * Once the Activity is finished, end the process, after saves still being copied back to
     * their documents. Two reasons: winit's event loop can't be created twice in one process, so
     * a later launch in this process would fail; and GameActivity's onDestroy waits for the Rust
     * thread to acknowledge the destroy, which winit's Android backend doesn't do once eframe's
     * loop has stopped, so calling it hangs the main thread (and every later launch with it).
     */
    @Override
    protected void onDestroy() {
        if (isFinishing()) {
            checkSync();
            io.shutdown();
            try {
                if (!io.awaitTermination(20, TimeUnit.SECONDS)) {
                    Log.w(TAG, "saves still copying after 20 s; ending anyway");
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            Log.i(TAG, "finished; ending the process");
            android.os.Process.killProcess(android.os.Process.myPid());
        }
        super.onDestroy();
    }

    @Override
    protected void onSaveInstanceState(Bundle out) {
        super.onSaveInstanceState(out);
        out.putLong("pendingId", pendingId);
        out.putString("pendingName", pendingName);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        handleSentDocument(intent);
    }

    private void hideSystemBars() {
        WindowInsetsControllerCompat c = WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        c.hide(WindowInsetsCompat.Type.systemBars());
        c.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    }

    // ---- Pen ------------------------------------------------------------------------------

    @Override
    public boolean dispatchTouchEvent(MotionEvent e) {
        // Some Samsung devices report an S Pen touching with its side button held as their own
        // actions (211 down, 212 up, 213 move, 214 cancel), which nothing downstream knows: the
        // pen then did nothing at all while the button was held. Turn them into Android's own
        // and remember the button, for the pen buttons' settings.
        int samsung = e.getActionMasked() - SAMSUNG_PEN_DOWN;
        sideButtonHeld = samsung >= 0 && samsung <= 3;
        if (sideButtonHeld) {
            int[] standard = {MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_CANCEL};
            e.setAction(standard[samsung] | (e.getAction() & MotionEvent.ACTION_POINTER_INDEX_MASK));
        }
        int action = e.getActionMasked();
        if (action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_POINTER_DOWN) {
            int i = e.getActionIndex();
            if (isPen(e.getToolType(i))) {
                // Pen samples as the digitizer reports them, not batched to the next frame.
                getWindow().getDecorView().requestUnbufferedDispatch(e);
            }
            try {
                nativePointer(e.getPointerId(i), e.getToolType(i), buttonsOf(e));
            } catch (UnsatisfiedLinkError err) {
                // The library isn't loaded: every pointer counts as a finger.
            }
        }
        inkEvent(e);
        forwardPen(e);
        return super.dispatchTouchEvent(e);
    }

    // ---- Low-latency ink ----------------------------------------------------------------------
    //
    // Touch Input › Low latency ink: the stroke being drawn is drawn here at once, from each touch
    // sample, on a front-buffered layer over the canvas (Android draws it straight to the screen,
    // without waiting for the app's next frame). The app draws the real stroke underneath as
    // usual; once it has drawn the finished stroke, this one is cleared. The app says what to draw
    // with when the touch goes down (nativeInkStart); colour 0 means no ink for this touch.

    private static native int nativeInkStart(float x, float y, boolean finger);
    private static native float nativeInkWidth(float pressure);
    private static native float nativeInkCanvas(int side);
    private static native long nativeInkEnd();
    private static native long nativeInkIdle();

    /** A segment of the stroke: from (x0, y0) to (x1, y1), this wide. */
    private static final class InkSegment {
        final float x0, y0, x1, y1, width;
        InkSegment(float x0, float y0, float x1, float y1, float width) {
            this.x0 = x0; this.y0 = y0; this.x1 = x1; this.y1 = y1; this.width = width;
        }
    }

    private androidx.graphics.lowlatency.CanvasFrontBufferedRenderer<InkSegment> ink;
    private final android.graphics.Paint inkPaint = new android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG);
    private final android.graphics.RectF inkClip = new android.graphics.RectF();
    /** The pointer drawing the stroke, or -1. */
    private int inkPointer = -1;
    private float inkX, inkY;
    /** Bumped for every stroke, so a pending clear doesn't clear the next one. */
    private int inkStroke = 0;

    private void inkEvent(MotionEvent e) {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) {
            return;
        }
        int action = e.getActionMasked();
        if (action == MotionEvent.ACTION_DOWN) {
            int color;
            try {
                color = nativeInkStart(e.getX(), e.getY(), !isPen(e.getToolType(0)));
            } catch (UnsatisfiedLinkError err) {
                color = 0;
            }
            if (color == 0 || !inkReady()) {
                inkPointer = -1;
                return;
            }
            inkStroke++;
            inkPaint.setColor(color);
            inkPaint.setStyle(android.graphics.Paint.Style.STROKE);
            inkPaint.setStrokeCap(android.graphics.Paint.Cap.ROUND);
            inkClip.set(nativeInkCanvas(0), nativeInkCanvas(1), nativeInkCanvas(2), nativeInkCanvas(3));
            inkPointer = e.getPointerId(0);
            inkX = e.getX();
            inkY = e.getY();
            inkTo(e.getX(), e.getY(), e.getPressure());
        } else if (inkPointer < 0) {
            return;
        } else if (action == MotionEvent.ACTION_MOVE) {
            int i = e.findPointerIndex(inkPointer);
            if (i < 0) {
                return;
            }
            for (int h = 0; h < e.getHistorySize(); h++) {
                inkTo(e.getHistoricalX(i, h), e.getHistoricalY(i, h), e.getHistoricalPressure(i, h));
            }
            inkTo(e.getX(i), e.getY(i), e.getPressure(i));
        } else if (action == MotionEvent.ACTION_UP) {
            inkPointer = -1;
            long idle;
            try {
                idle = nativeInkEnd();
            } catch (UnsatisfiedLinkError err) {
                idle = Long.MAX_VALUE - 2;
            }
            // Cleared once the app has drawn a frame or two with the stroke finished.
            inkClearAfter(inkStroke, idle + 2, SystemClock.uptimeMillis() + 500);
        } else if (action == MotionEvent.ACTION_CANCEL || action == MotionEvent.ACTION_POINTER_DOWN) {
            // Cancelled, or a second finger: a gesture, not a stroke.
            inkPointer = -1;
            ink.clear();
        }
    }

    private boolean inkReady() {
        if (ink == null) {
            try {
                ink = new androidx.graphics.lowlatency.CanvasFrontBufferedRenderer<>(mSurfaceView, new androidx.graphics.lowlatency.CanvasFrontBufferedRenderer.Callback<InkSegment>() {
                    @Override
                    public void onDrawFrontBufferedLayer(android.graphics.Canvas canvas, int width, int height, InkSegment s) {
                        canvas.save();
                        canvas.clipRect(inkClip);
                        inkPaint.setStrokeWidth(s.width);
                        canvas.drawLine(s.x0, s.y0, s.x1, s.y1, inkPaint);
                        canvas.restore();
                    }

                    @Override
                    public void onDrawMultiBufferedLayer(android.graphics.Canvas canvas, int width, int height, java.util.Collection<? extends InkSegment> segments) {
                        // Never committed: the app draws the real stroke.
                        canvas.drawColor(android.graphics.Color.TRANSPARENT, android.graphics.PorterDuff.Mode.CLEAR);
                    }
                });
            } catch (RuntimeException | LinkageError err) {
                Log.w(TAG, "low latency ink unavailable", err);
                return false;
            }
        }
        return ink.isValid();
    }

    private void inkTo(float x, float y, float pressure) {
        float width = nativeInkWidth(pressure);
        if (width > 0f) {
            ink.renderFrontBufferedLayer(new InkSegment(inkX, inkY, x, y, width));
        }
        inkX = x;
        inkY = y;
    }

    private void inkClearAfter(int stroke, long idle, long deadline) {
        android.view.Choreographer.getInstance().postFrameCallback(t -> {
            if (stroke != inkStroke || inkPointer >= 0) {
                // A new stroke started: it clears itself when it's done.
                return;
            }
            long now;
            try {
                now = nativeInkIdle();
            } catch (UnsatisfiedLinkError err) {
                now = idle;
            }
            if (now >= idle || SystemClock.uptimeMillis() >= deadline) {
                ink.clear();
            } else {
                inkClearAfter(stroke, idle, deadline);
            }
        });
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent e) {
        sideButtonHeld = false;
        forwardPen(e);
        return super.dispatchGenericMotionEvent(e);
    }

    /** Samsung's first S Pen action with the side button held (see dispatchTouchEvent). */
    private static final int SAMSUNG_PEN_DOWN = 211;
    /** The event being dispatched came as one of Samsung's side-button actions. */
    private boolean sideButtonHeld;

    /** The event's buttons, with the side button when Samsung reported it as its own action. */
    private int buttonsOf(MotionEvent e) {
        return e.getButtonState() | (sideButtonHeld ? MotionEvent.BUTTON_STYLUS_PRIMARY : 0);
    }

    private static boolean isPen(int tool) {
        return tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER;
    }

    /** The pointer whose pen state this event reports: a pen pointer if any (a palm resting
     * beside the pen doesn't end its stroke), else the event's own pointer. */
    private static int penPointer(MotionEvent e) {
        int action = e.getActionMasked();
        if (action == MotionEvent.ACTION_POINTER_DOWN || action == MotionEvent.ACTION_POINTER_UP) {
            return e.getActionIndex();
        }
        for (int i = 0; i < e.getPointerCount(); i++) {
            if (isPen(e.getToolType(i))) {
                return i;
            }
        }
        return 0;
    }

    /** Milliseconds since an event happened (both on the uptime clock), for the latency trace. */
    private static int age(long now, long eventTime) {
        return (int) Math.max(0, Math.min(now - eventTime, 10_000));
    }

    private void forwardPen(MotionEvent e) {
        // A marker for Perfetto traces (free when no trace is running): lines pen delivery up with frames.
        Trace.beginSection("photocraft:pen");
        try {
            int action = e.getActionMasked();
            int p = penPointer(e);
            int tool = e.getToolType(p);
            int buttons = buttonsOf(e);
            long now = SystemClock.uptimeMillis();
            if (action == MotionEvent.ACTION_POINTER_DOWN || action == MotionEvent.ACTION_POINTER_UP) {
                // Another finger coming or going says nothing about the pen.
                if (!isPen(tool)) {
                    return;
                }
            }
            if (action == MotionEvent.ACTION_MOVE && isPen(tool)) {
                // Samples the digitizer reported between two frames, oldest first.
                for (int h = 0; h < e.getHistorySize(); h++) {
                    nativePen(action, tool, buttons, e.getHistoricalPressure(p, h),
                            e.getHistoricalAxisValue(MotionEvent.AXIS_TILT, p, h), e.getHistoricalOrientation(p, h),
                            e.getHistoricalX(p, h), e.getHistoricalY(p, h), age(now, e.getHistoricalEventTime(h)),
                            e.getHistoricalSize(p, h), e.getHistoricalTouchMajor(p, h));
                }
            }
            // Fingers too: their contact size can stand in for pressure (Touch Input).
            nativePen(action, tool, buttons, e.getPressure(p), e.getAxisValue(MotionEvent.AXIS_TILT, p), e.getOrientation(p),
                    e.getX(p), e.getY(p), age(now, e.getEventTime()), e.getSize(p), e.getTouchMajor(p));
        } catch (UnsatisfiedLinkError err) {
            // The library isn't loaded (it failed to load): the pen paints like a finger.
        } catch (RuntimeException err) {
            Log.w(TAG, "pen", err);
        } finally {
            Trace.endSection();
        }
    }

    // ---- Document pickers -------------------------------------------------------------------

    private void checkRequest() throws Exception {
        File f = new File(bridge, "request.json");
        if (pendingId != -1 || !f.isFile()) {
            return;
        }
        JSONObject r = new JSONObject(readText(f));
        //noinspection ResultOfMethodCallIgnored
        f.delete();
        long id = r.getLong("id");
        String kind = r.getString("kind");
        Intent intent;
        int code;
        if ("open".equals(kind)) {
            intent = new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*");
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, r.optBoolean("multiple", false));
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                    | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            code = REQ_OPEN;
        } else if ("save".equals(kind)) {
            pendingName = r.optString("name", "Untitled.png");
            intent = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                    .setType(r.optString("mime", "application/octet-stream"))
                    .putExtra(Intent.EXTRA_TITLE, pendingName);
            code = REQ_SAVE;
        } else {
            answerError(id, "unknown request " + kind);
            return;
        }
        pendingId = id;
        try {
            startActivityForResult(intent, code);
        } catch (ActivityNotFoundException e) {
            pendingId = -1;
            answerError(id, "no document picker on this device");
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != REQ_OPEN && requestCode != REQ_SAVE) {
            return;
        }
        final long id = pendingId;
        pendingId = -1;
        if (id == -1) {
            return;
        }
        if (resultCode != Activity.RESULT_OK || data == null) {
            answerCancelled(id);
            return;
        }
        if (requestCode == REQ_OPEN) {
            final List<Uri> uris = new ArrayList<>();
            ClipData clip = data.getClipData();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount(); i++) {
                    uris.add(clip.getItemAt(i).getUri());
                }
            } else if (data.getData() != null) {
                uris.add(data.getData());
            }
            final int flags = data.getFlags();
            io.execute(() -> copyIn(id, uris, flags));
        } else {
            final Uri uri = data.getData();
            if (uri == null) {
                answerCancelled(id);
                return;
            }
            // The app picks the format from the extension it asked for. Some providers report
            // a name without it, or no name at all (Downloads on some devices: an id such as
            // "msf:1234"), and "Untitled-1" can't be saved as anything: keep the requested
            // name unless the provider's only differs in its base (say "Untitled-1 (1).psd").
            String provided = displayName(uri, pendingName);
            String name = extensionOf(provided).equals(extensionOf(pendingName)) ? provided : pendingName;
            if (!name.equals(provided)) {
                Log.i(TAG, "save: the provider named the document \"" + provided + "\"; writing \"" + name + "\"");
            }
            File dest = new File(bridge, "out/" + id + "/" + safeName(name));
            //noinspection ResultOfMethodCallIgnored
            dest.getParentFile().mkdirs();
            remember(dest, uri);
            try {
                answer(new JSONObject().put("id", id).put("path", dest.getAbsolutePath()));
            } catch (Exception e) {
                Log.e(TAG, "save answer", e);
            }
        }
    }

    /** Copy picked documents into the bridge inbox and answer with their paths. */
    private void copyIn(long id, List<Uri> uris, int flags) {
        JSONArray paths = new JSONArray();
        String error = null;
        for (Uri uri : uris) {
            boolean writable = keepAccess(uri, flags);
            File dest = new File(bridge, "inbox/" + id + "/" + safeName(nameWithExtension(uri)));
            //noinspection ResultOfMethodCallIgnored
            dest.getParentFile().mkdirs();
            try (InputStream in = getContentResolver().openInputStream(uri);
                 OutputStream out = new FileOutputStream(dest)) {
                if (in == null) {
                    throw new IOException("the provider returned no data");
                }
                copy(in, out);
                paths.put(dest.getAbsolutePath());
                if (writable) {
                    remember(dest, uri);
                } else {
                    forget(dest);
                }
            } catch (Exception e) {
                Log.e(TAG, "open " + uri, e);
                error = e.getMessage();
            }
        }
        try {
            JSONObject a = new JSONObject().put("id", id).put("paths", paths);
            if (error != null) {
                a.put("error", error);
            }
            answer(a);
        } catch (Exception e) {
            Log.e(TAG, "open answer", e);
        }
    }

    /** Keep access to an opened document across restarts; true when it may be written back. */
    private boolean keepAccess(Uri uri, int flags) {
        boolean write = (flags & Intent.FLAG_GRANT_WRITE_URI_PERMISSION) != 0;
        try {
            getContentResolver().takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION
                    | (write ? Intent.FLAG_GRANT_WRITE_URI_PERMISSION : 0));
        } catch (SecurityException e) {
            // Not persistable: the grant lasts while the app runs, which is enough to save back.
        }
        return write;
    }

    // ---- Leaving ---------------------------------------------------------------------------

    /** File › Exit (bridge `quit`): finish once pending saves are queued (see onDestroy). */
    private void checkQuit() {
        File f = new File(bridge, "quit");
        if (!f.exists()) {
            return;
        }
        //noinspection ResultOfMethodCallIgnored
        f.delete();
        checkSync();
        finish();
    }

    /**
     * The system clipboard for text fields: `clip-set.txt` (written by the app) is copied to it,
     * and `clip-get` asks for its text, answered in `clip.txt`. Read only when the app asks (a
     * paste), and the files are removed as soon as they're read.
     */
    private void checkClipboard() throws IOException {
        android.content.ClipboardManager cm = (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (cm == null) {
            return;
        }
        File set = new File(bridge, "clip-set.txt");
        if (set.isFile()) {
            String text = readText(set);
            //noinspection ResultOfMethodCallIgnored
            set.delete();
            cm.setPrimaryClip(ClipData.newPlainText("PhotoCraft", text));
        }
        File get = new File(bridge, "clip-get");
        if (get.exists() && get.delete()) {
            String text = "";
            ClipData clip = cm.getPrimaryClip();
            if (clip != null && clip.getItemCount() > 0) {
                CharSequence t = clip.getItemAt(0).coerceToText(this);
                if (t != null) {
                    text = t.toString();
                }
            }
            File tmp = new File(bridge, "clip.txt.tmp");
            try (OutputStream out = new FileOutputStream(tmp)) {
                out.write(text.getBytes(StandardCharsets.UTF_8));
            }
            if (!tmp.renameTo(new File(bridge, "clip.txt"))) {
                Log.w(TAG, "clipboard: rename failed");
            }
        }
    }

    /** Help › Check for Updates… (bridge `check-update`). */
    private void checkUpdateRequest() {
        File f = new File(bridge, "check-update");
        if (f.exists() && f.delete()) {
            updater.check(true);
        }
    }

    // ---- Writing back ----------------------------------------------------------------------

    private void checkSync() {
        File[] files = new File(bridge, "sync").listFiles((dir, name) -> name.endsWith(".json"));
        if (files == null) {
            return;
        }
        for (File f : files) {
            try {
                String path = new JSONObject(readText(f)).getString("path");
                //noinspection ResultOfMethodCallIgnored
                f.delete();
                final File local = new File(path);
                final String uri = documents().getString(local.getAbsolutePath(), null);
                if (uri == null) {
                    toast("Saved inside PhotoCraft only: this document can't be written back. Use File › Save As to choose where to save.");
                    continue;
                }
                io.execute(() -> copyOut(local, Uri.parse(uri)));
            } catch (Exception e) {
                Log.e(TAG, "sync " + f, e);
                //noinspection ResultOfMethodCallIgnored
                f.delete();
            }
        }
    }

    private void copyOut(File local, Uri uri) {
        try (InputStream in = new FileInputStream(local);
             OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
            if (out == null) {
                throw new IOException("the provider refused the write");
            }
            copy(in, out);
            Log.i(TAG, "saved " + local.getName() + " to " + uri);
        } catch (Exception e) {
            Log.e(TAG, "save " + uri, e);
            toast("Couldn't save " + local.getName() + ": " + e.getMessage());
        }
    }

    private SharedPreferences documents() {
        return getSharedPreferences("documents", MODE_PRIVATE);
    }

    private void remember(File local, Uri uri) {
        documents().edit().putString(local.getAbsolutePath(), uri.toString()).apply();
    }

    private void forget(File local) {
        documents().edit().remove(local.getAbsolutePath()).apply();
    }

    // ---- Documents from other apps ---------------------------------------------------------

    private void handleSentDocument(Intent intent) {
        if (intent == null) {
            return;
        }
        Uri uri = null;
        if (Intent.ACTION_VIEW.equals(intent.getAction())) {
            uri = intent.getData();
        } else if (Intent.ACTION_SEND.equals(intent.getAction())) {
            uri = intent.getParcelableExtra(Intent.EXTRA_STREAM);
        }
        if (uri == null) {
            return;
        }
        // Handled: a recreated Activity mustn't open it again.
        setIntent(new Intent(Intent.ACTION_MAIN));
        final Uri from = uri;
        final String name = (incomingSeq++) + "-" + safeName(nameWithExtension(uri));
        io.execute(() -> {
            File dir = new File(bridge, "incoming");
            //noinspection ResultOfMethodCallIgnored
            dir.mkdirs();
            File tmp = new File(dir, name + ".tmp");
            try (InputStream in = getContentResolver().openInputStream(from);
                 OutputStream out = new FileOutputStream(tmp)) {
                if (in == null) {
                    throw new IOException("the provider returned no data");
                }
                copy(in, out);
            } catch (Exception e) {
                Log.e(TAG, "incoming " + from, e);
                toast("Couldn't open the document: " + e.getMessage());
                //noinspection ResultOfMethodCallIgnored
                tmp.delete();
                return;
            }
            if (!tmp.renameTo(new File(dir, name))) {
                Log.e(TAG, "incoming: rename failed");
            }
        });
    }

    // ---- Helpers ---------------------------------------------------------------------------

    private void answerCancelled(long id) {
        try {
            answer(new JSONObject().put("id", id).put("cancelled", true));
        } catch (Exception e) {
            Log.e(TAG, "answer", e);
        }
    }

    private void answerError(long id, String error) {
        try {
            answer(new JSONObject().put("id", id).put("cancelled", true).put("error", error));
        } catch (Exception e) {
            Log.e(TAG, "answer", e);
        }
    }

    /** Write answer.json under a temporary name, then rename, so Rust never reads half of it. */
    private synchronized void answer(JSONObject a) throws IOException {
        File tmp = new File(bridge, "answer.json.tmp");
        try (OutputStream out = new FileOutputStream(tmp)) {
            out.write(a.toString().getBytes(StandardCharsets.UTF_8));
        }
        if (!tmp.renameTo(new File(bridge, "answer.json"))) {
            throw new IOException("couldn't publish the answer");
        }
    }

    private String displayName(Uri uri, String fallback) {
        try (Cursor c = getContentResolver().query(uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) {
                return c.getString(0);
            }
        } catch (Exception e) {
            Log.w(TAG, "display name", e);
        }
        String last = uri.getLastPathSegment();
        return last != null ? last : fallback;
    }

    /** A document's name with an extension: saving it again (Ctrl+S) picks the format from it.
     * Without one in the provider's name, the extension of its MIME type is added. */
    private String nameWithExtension(Uri uri) {
        String name = displayName(uri, "document");
        if (!extensionOf(name).isEmpty()) {
            return name;
        }
        String type = null;
        try {
            type = getContentResolver().getType(uri);
        } catch (Exception e) {
            Log.w(TAG, "type", e);
        }
        String ext = type == null ? null : MimeTypeMap.getSingleton().getExtensionFromMimeType(type);
        if (ext == null && "image/vnd.adobe.photoshop".equals(type)) {
            ext = "psd";
        }
        Log.i(TAG, "open: \"" + name + "\" has no extension; type " + type + " -> " + ext);
        return ext == null ? name : name + "." + ext;
    }

    /** The extension of a file name, lower case; "" for none. */
    static String extensionOf(String name) {
        int dot = name.lastIndexOf('.');
        return dot > 0 && dot < name.length() - 1 ? name.substring(dot + 1).toLowerCase(java.util.Locale.ROOT) : "";
    }

    static String safeName(String name) {
        String s = name.replaceAll("[\\\\/:*?\"<>|\\p{Cntrl}]", "_").trim();
        while (s.startsWith(".")) {
            s = s.substring(1);
        }
        return s.isEmpty() ? "document" : s;
    }

    private static String readText(File f) throws IOException {
        try (InputStream in = new FileInputStream(f)) {
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            copy(in, out);
            return out.toString("UTF-8");
        }
    }

    private static void copy(InputStream in, OutputStream out) throws IOException {
        byte[] buf = new byte[1 << 16];
        int n;
        while ((n = in.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
    }

    private void toast(String text) {
        runOnUiThread(() -> Toast.makeText(this, text, Toast.LENGTH_LONG).show());
    }
}
