package io.github.jramirez_codes.nex;

import android.app.Activity;
import android.content.Context;
import android.text.InputType;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

/**
 * The view the on-screen keyboard types into. NativeActivity's own view gives the keyboard no
 * InputConnection, so text a keyboard commits in one go (a paste from its clipboard history, a
 * suggestion, an emoji) reaches native code as a key event whose characters the NDK can't read.
 * This view's connection hands committed text to nex instead (`commit`). Keys the keyboard
 * sends as key events (letters, Enter, Backspace) still go to nex's input queue as before,
 * since NativeActivity owns the window's input.
 *
 * Loaded at run time from the dex built into nex (see `platform/android/keyboard.rs`).
 */
public final class NexInput extends View {
    private static NexInput view;

    /** Text the keyboard committed, for nex to type into whatever has focus. */
    static native void commit(String text);

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
        // TYPE_NULL: no autocorrect or composing; keyboards send plain keys as key events.
        out.inputType = InputType.TYPE_NULL;
        out.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN | EditorInfo.IME_FLAG_NO_EXTRACT_UI;
        return new BaseInputConnection(this, false) {
            @Override
            public boolean commitText(CharSequence text, int newCursorPosition) {
                if (text != null && text.length() > 0) {
                    commit(text.toString());
                }
                return true;
            }

            @Override
            public boolean deleteSurroundingText(int before, int after) {
                for (int i = 0; i < before; i++) {
                    sendKeyEvent(new KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL));
                    sendKeyEvent(new KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL));
                }
                return true;
            }
        };
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
