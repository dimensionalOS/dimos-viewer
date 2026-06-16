## ADDED requirements

### Requirement: configured native web page view
The system SHALL provide a native-only Web Page View that displays a configured webpage inline in the viewer layout.

#### Scenario: native view displays configured page
- **WHEN** a native viewer layout contains a Web Page View configured with `https://example.com`
- **THEN** the viewer displays that page inline in the view area

#### Scenario: web viewer reports unsupported view
- **WHEN** the web viewer renders a layout containing a Web Page View
- **THEN** the viewer displays a clear unsupported status for that view

### Requirement: blueprint-owned configuration
The system SHALL store Web Page View configuration as blueprint/view state with a required URL and a `show_navigation_controls` setting.

#### Scenario: blueprint preconfigures a web page view
- **WHEN** blueprint state contains a Web Page View with a valid URL and navigation controls setting
- **THEN** the viewer creates the view with those configured values

#### Scenario: manual creation starts unconfigured
- **WHEN** a user manually adds a Web Page View without setting a URL
- **THEN** the viewer displays a Rerun-side status explaining that no URL is configured

### Requirement: no data-driven spawning
The system SHALL make Web Page Views manually creatable and blueprint-preconfigurable without auto-spawning them from logged data.

#### Scenario: logged entities do not suggest web page view
- **WHEN** the viewer computes data-driven view suggestions from logged entities
- **THEN** the suggestion set excludes Web Page View unless it was explicitly created or configured

### Requirement: HTTP URL policy
The system SHALL validate configured Web Page View URLs and load only `http://` and `https://` schemes.

#### Scenario: HTTPS URL loads
- **WHEN** a Web Page View is configured with `https://example.com`
- **THEN** the viewer attempts to load the URL in the native webview

#### Scenario: local HTTP URL loads
- **WHEN** a Web Page View is configured with `http://localhost:3000`
- **THEN** the viewer attempts to load the URL in the native webview

#### Scenario: unsupported scheme is rejected
- **WHEN** a Web Page View is configured with `file:///tmp/report.html`
- **THEN** the viewer displays a Rerun-side status explaining that only `http` and `https` URLs are supported

### Requirement: automatic loading
The system SHALL automatically load a valid configured Web Page View URL without requiring an additional confirmation action.

#### Scenario: valid URL autoloads
- **WHEN** a Web Page View becomes visible with a valid configured URL
- **THEN** the native webview starts loading that URL automatically

### Requirement: per-view webview instances
The system SHALL create and manage one native webview instance for each Web Page View instance.

#### Scenario: multiple views render independently
- **WHEN** a layout contains two Web Page Views with different valid URLs
- **THEN** the viewer maintains two independent native webview instances and displays both pages in their respective view areas

### Requirement: webview lifecycle preserves hidden view state
The system SHALL keep a Web Page View's native webview instance alive while the Rerun view exists, including when temporarily hidden.

#### Scenario: hidden view keeps page state
- **WHEN** a Web Page View is hidden behind a tab and later shown again
- **THEN** the viewer reuses the existing webview instance rather than reloading the configured URL from scratch

#### Scenario: removed view destroys instance
- **WHEN** a Web Page View is removed from the layout
- **THEN** the viewer destroys the corresponding native webview instance

### Requirement: browser-like navigation
The system SHALL allow runtime browser navigation inside the Web Page View while keeping the configured URL unchanged.

#### Scenario: link navigation does not change blueprint URL
- **WHEN** a user clicks a link inside a Web Page View and the embedded page navigates to a different URL
- **THEN** the blueprint-configured URL remains the original configured URL

#### Scenario: home returns to configured URL
- **WHEN** navigation controls are visible and the user activates Home after navigating away
- **THEN** the webview navigates back to the configured URL

### Requirement: optional navigation controls
The system SHALL provide lightweight navigation controls for Web Page Views and allow them to be hidden by configuration.

#### Scenario: navigation controls visible by default
- **WHEN** a Web Page View is configured without specifying `show_navigation_controls`
- **THEN** the viewer displays back, forward, reload, home, and URL display controls

#### Scenario: navigation controls hidden
- **WHEN** a Web Page View is configured with `show_navigation_controls` set to false
- **THEN** the viewer hides the navigation controls and gives the webview the available view area

### Requirement: shared embedded browser session
The system SHALL use a shared embedded browser session/profile for Web Page Views by default.

#### Scenario: session state shared across views
- **WHEN** two Web Page Views load pages from the same origin
- **THEN** the embedded browser backend uses the shared default session/profile for both views

### Requirement: rerun-side failure reporting
The system SHALL report configuration and backend failures using Rerun-side status UI outside the embedded webpage.

#### Scenario: backend creation fails
- **WHEN** the native webview backend cannot create a webview instance
- **THEN** the Web Page View displays a clear Rerun-side failure message instead of crashing the viewer

#### Scenario: invalid URL is configured
- **WHEN** the configured URL cannot be parsed as a URL
- **THEN** the Web Page View displays a clear Rerun-side invalid URL message
