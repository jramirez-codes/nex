package io.github.jramirez_codes.nex;

import android.app.Activity;
import android.content.Context;
import android.graphics.Rect;
import android.text.InputType;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewTreeObserver;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

/**
 * The view the on-screen keyboard types into. NativeActivity's own view gives the keyboard no
 * InputConnection, so text a keyboard commits in one go (a paste from its clipboard history, a
 * suggestion, an emoji) reaches native code as a key event whose characters the NDK can't read.
 * This view's connection hands text to nex instead (`type`, `commit`). Keys the keyboard sends
 * as key events (Enter, Backspace on an empty field) still go to nex's input queue as before,
 * since NativeActivity owns the window's input.
 *
 * The view says it is a text field, which is what keyboards want before they offer glide
 * (swipe) typing. A glided word arrives as composing text that the keyboard keeps revising
 * until it settles; nex types it into the terminal as it goes, taking back with Backspace what
 * a revision changed (`sent` is what is on the terminal so far), and nex types a space after
 * it once it's committed (`wordDone`). The field itself is always left empty, so the keyboard
 * never tries to edit text that is already in the terminal, and Backspace with nothing
 * composing arrives as a plain key event.
 *
 * Loaded at run time from the dex built into nex (see `platform/android/keyboard.rs`).
 */
public final class NexInput extends View {
    private static NexInput view;

    /** Take back `delete` characters, then type `text` (typing, glide, a suggestion);
     *  `composing` when it's the word the keyboard is still composing. */
    static native void type(int delete, String text, boolean composing);

    /** The keyboard `committed` the word it was composing, or gave it up. nex types a space
     *  after a glided word, since the empty field leaves the keyboard no word to space after. */
    static native void wordDone(boolean committed);

    /** Text the keyboard committed in one go that isn't typing: a paste from its clipboard. */
    static native void commit(String text);

    /** The on-screen keyboard came up or went away (including by the back gesture). */
    static native void imeVisible(boolean visible);

    /** What nex was last told about the keyboard, so it hears only of changes. */
    private static Boolean lastVisible;

    /** The composing text already typed into the terminal. */
    private static String sent = "";

    private NexInput(Context context) {
        super(context);
        setFocusable(true);
        setFocusableInTouchMode(true);
    }

    @Override
    public boolean onCheckIsTextEditor() {
        return true;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo out) {
        // A text field, so keyboards offer glide typing; no suggestions (and, by leaving out
        // AUTO_CORRECT, no autocorrect), which only get in the way of typing commands.
        out.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
        out.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN | EditorInfo.IME_FLAG_NO_EXTRACT_UI
                | EditorInfo.IME_ACTION_NONE;
        out.initialSelStart = 0;
        out.initialSelEnd = 0;
        sent = "";
        wordDone(false);
        return new BaseInputConnection(this, false) {
            @Override
            public boolean setComposingText(CharSequence text, int newCursorPosition) {
                retype(text == null ? "" : text.toString(), true);
                return true;
            }

            @Override
            public boolean commitText(CharSequence text, int newCursorPosition) {
                String t = text == null ? "" : text.toString();
                if (sent.isEmpty() && isPaste(t)) {
                    commit(t);
                } else {
                    retype(t, false);
                }
                sent = "";
                wordDone(true);
                return true;
            }

            @Override
            public boolean finishComposingText() {
                sent = "";
                wordDone(true);
                return true;
            }

            @Override
            public boolean setComposingRegion(int start, int end) {
                // The field is empty, so there is no earlier text to pick up again.
                sent = "";
                wordDone(true);
                return true;
            }

            @Override
            public CharSequence getTextBeforeCursor(int n, int flags) {
                return "";
            }

            @Override
            public CharSequence getTextAfterCursor(int n, int flags) {
                return "";
            }

            @Override
            public boolean deleteSurroundingText(int before, int after) {
                deleteBack(before);
                return true;
            }

            @Override
            public boolean deleteSurroundingTextInCodePoints(int before, int after) {
                deleteBack(before);
                return true;
            }

            @Override
            public boolean performEditorAction(int action) {
                sendKeyEvent(new KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER));
                sendKeyEvent(new KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER));
                return true;
            }
        };
    }

    /** Make the terminal show `text` where it shows `sent`: Backspace over what differs, then
     *  type the rest. */
    private static void retype(String text, boolean composing) {
        int common = 0;
        int max = Math.min(sent.length(), text.length());
        while (common < max && sent.charAt(common) == text.charAt(common)) {
            common++;
        }
        // Don't split a surrogate pair.
        if (common > 0 && Character.isHighSurrogate(sent.charAt(common - 1))) {
            common--;
        }
        int delete = sent.codePointCount(common, sent.length());
        String add = text.substring(common);
        if (delete > 0 || !add.isEmpty()) {
            type(delete, add, composing);
        }
        sent = text;
    }

    /** Backspace `count` characters, from what is composing first. */
    private static void deleteBack(int count) {
        if (count <= 0) {
            return;
        }
        int composed = sent.codePointCount(0, sent.length());
        int fromSent = Math.min(count, composed);
        if (fromSent > 0) {
            sent = sent.substring(0, sent.offsetByCodePoints(0, composed - fromSent));
        }
        type(count, "", false);
    }

    /** Whether committed text is a paste rather than typing: more than one character that
     *  isn't a single word (a glided word or a suggestion, maybe with a space after it). */
    private static boolean isPaste(String t) {
        if (t.codePointCount(0, t.length()) <= 1) {
            return false;
        }
        return !t.matches("[\\p{L}\\p{N}'\u2019-]+ ?");
    }

    /** Haptic feedback (`HapticFeedbackConstants`), as Android's own keys and controls give. */
    public static void haptic(final Activity activity, final int kind) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (view == null) {
                    return;
                }
                // CONFIRM (16) is new in Android 11; a long press's buzz stands in before it.
                int k = kind == 16 && android.os.Build.VERSION.SDK_INT < 30 ? 0 : kind;
                view.performHapticFeedback(k);
            }
        });
    }

    /** Whether the app may record from the microphone. */
    public static boolean hasMic(Activity activity) {
        return activity.checkSelfPermission(android.Manifest.permission.RECORD_AUDIO)
                == android.content.pm.PackageManager.PERMISSION_GRANTED;
    }

    /** Ask the user to let the app use the microphone. NativeActivity doesn't pass the answer
     *  on, so nex checks {@link #hasMic} again the next time it needs the mic. */
    public static void askMic(final Activity activity) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                activity.requestPermissions(new String[] { android.Manifest.permission.RECORD_AUDIO }, 1);
            }
        });
    }

    /** Add the (invisible, 1 px) view to the activity and give it focus. */
    public static void install(final Activity activity) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (view != null) {
                    return;
                }
                view = new NexInput(activity);
                activity.addContentView(view, new ViewGroup.LayoutParams(1, 1));
                view.requestFocus();
                watchKeyboard(activity.getWindow().getDecorView());
            }
        });
    }

    /**
     * Tell nex when the keyboard comes up or goes away. WindowInsets can say so directly only
     * from API 30; the visible part of the window works on every version: the keyboard is up
     * when it covers more than 15% of the screen (the navigation bar alone never does).
     */
    private static void watchKeyboard(final View root) {
        root.getViewTreeObserver().addOnGlobalLayoutListener(new ViewTreeObserver.OnGlobalLayoutListener() {
            @Override
            public void onGlobalLayout() {
                Rect free = new Rect();
                root.getWindowVisibleDisplayFrame(free);
                int height = root.getRootView().getHeight();
                if (height <= 0) {
                    return;
                }
                boolean visible = height - free.bottom > height * 0.15;
                if (lastVisible == null || lastVisible != visible) {
                    lastVisible = visible;
                    imeVisible(visible);
                }
            }
        });
    }

    /** Show or hide the on-screen keyboard, typing into this view. */
    public static void showKeyboard(final Activity activity, final boolean show) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (view == null) {
                    return;
                }
                InputMethodManager imm = (InputMethodManager) activity.getSystemService(Context.INPUT_METHOD_SERVICE);
                if (show) {
                    view.requestFocus();
                    imm.showSoftInput(view, 0);
                } else {
                    imm.hideSoftInputFromWindow(view.getWindowToken(), 0);
                }
            }
        });
    }
}
