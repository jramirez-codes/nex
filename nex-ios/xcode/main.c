// The app's entry point: hand the main thread to nex (nex-ios/src/lib.rs), which hands it to
// UIKit. It never returns.
extern void nex_ios_main(void);

int main(int argc, char *argv[]) {
    (void)argc;
    (void)argv;
    nex_ios_main();
    return 0;
}
