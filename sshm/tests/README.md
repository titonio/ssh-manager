# Integration Tests for sshm

This directory contains integration tests for the SSH Connection Manager (sshm) TUI application.

## Testing Framework

The tests use **insta** for snapshot testing with Ratatui's `TestBackend`. This allows us to:

1. **Visual regression testing**: Capture the exact visual output of the TUI at different states
2. **Layout validation**: Verify that UI components render in the correct positions
3. **Content verification**: Ensure that text, headers, footers, and connection lists display correctly
4. **State testing**: Test different application modes (Normal, Add, Edit, Search, Help)

## Running Tests

### Run all integration tests:
```bash
cargo test --test integration_test
```

### Run specific test:
```bash
cargo test --test integration_test test_main_screen_layout
```

### Update snapshots:
When you make intentional UI changes, update the snapshots with:
```bash
cargo insta test --test integration_test --accept
```

### Review snapshot changes:
```bash
cargo insta review
```

## Test Coverage

The integration tests cover the following E2E use cases:

1. **Main Screen Layout** - Verifies the complete UI structure with header, search bar, connection list, and footer
2. **Connection List Display** - Tests that all connections are shown with correct formatting
3. **Selection Highlighting** - Validates that the selected item is properly highlighted
4. **Search Mode** - Tests the search functionality and filtered results display
5. **Help Mode** - Verifies the help popup displays correctly
6. **Add Connection Popup** - Tests the form for adding new connections
7. **Edit Connection Popup** - Validates the edit form with pre-filled data
8. **Empty State** - Tests the message shown when no connections exist
9. **Search Filtering** - Verifies that search filters connections correctly
10. **No Matches** - Tests the message when search returns no results
11. **Custom Port Display** - Validates connections with non-standard ports
12. **Folder Grouping** - Tests connections organized by folders
13. **Different Terminal Sizes** - Ensures UI adapts to different terminal dimensions
14. **Message Popup** - Tests temporary message notifications
15. **Input Field Navigation** - Validates form field navigation in add/edit modes

## Snapshot Files

Snapshot files are stored in `tests/snapshots/` with the naming convention:
`integration_test__<test_name>.snap`

Each snapshot contains the exact terminal output for that test case, including:
- All visible text
- UI borders and layout
- Selection indicators
- Mode-specific elements

## Adding New Tests

To add a new integration test:

1. Create a new test function in `tests/integration_test.rs`
2. Set up the app state for your test case
3. Render the UI using `terminal.draw(|f| app.render(f))`
4. Use `assert_snapshot!(backend)` to capture the output
5. Run `cargo insta test --accept` to create the snapshot

Example:
```rust
#[test]
fn test_new_feature() {
    let mut app = create_test_app();
    // Setup app state
    app.mode = AppMode::Add;
    
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    
    let backend = terminal.backend();
    assert_snapshot!(backend);
}
```

## CI/CD Integration

The tests are designed to run in headless CI environments. Add to your CI pipeline:

```yaml
test:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v2
    - name: Run tests
      run: cargo test --test integration_test
    - name: Review snapshots
      if: failure()
      run: cargo insta review
```

## Visual Validation

The snapshot tests automatically validate the TUI visual structure by:
- Comparing rendered output against baseline snapshots
- Detecting unintended UI changes
- Ensuring consistent layout across runs
- Verifying text content and positioning

For more advanced visual testing (e.g., Sixel graphics, complex animations), consider using **ratatui-testlib** (terminal-testlib), which provides PTY-based testing with real terminal emulation.
