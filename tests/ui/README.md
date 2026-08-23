# Zintl UI tests

The macOS UI tests build a dedicated Rust fixture app, package it as an app
bundle, and run XCUITest against it.

Run every UI test from the repository root:

```sh
./tests/ui/tools/scripts/test.sh
```

Run one test class:

```sh
./tests/ui/tools/scripts/test.sh \
  -only-testing:ZintlUITests/TextFieldTests
```

Rust fixture scenarios live under `tests/app/scenarios`. XCUITest sources live
under `tests/xcui`; new Swift files in that directory are discovered by
`tests/ZintlUITests.xcodeproj` automatically.
