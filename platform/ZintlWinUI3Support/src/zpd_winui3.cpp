#include "zpd_winui3.h"

#define NOMINMAX
#include <windows.h>
#undef GetCurrentTime
#include <MddBootstrap.h>

#include <algorithm>
#include <cctype>
#include <functional>
#include <memory>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include <winrt/Microsoft.UI.Dispatching.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.Controls.Primitives.h>
#include <winrt/Microsoft.UI.Xaml.Hosting.h>
#include <winrt/Microsoft.UI.Xaml.Input.h>
#include <winrt/Microsoft.UI.Xaml.Markup.h>
#include <winrt/Microsoft.UI.Xaml.Media.h>
#include <winrt/Microsoft.UI.Xaml.XamlTypeInfo.h>
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Microsoft.UI.Windowing.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.System.h>

using namespace winrt;
using namespace Microsoft::UI;
using namespace Microsoft::UI::Dispatching;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;
using namespace Microsoft::UI::Xaml::Markup;

thread_local std::string last_error;

std::string_view bytes(zpd_winui3_string value) {
  return {reinterpret_cast<const char*>(value.data), value.length};
}

hstring string_from(zpd_winui3_string value) {
  return to_hstring(bytes(value));
}

template <typename F>
int32_t status(F&& body) noexcept {
  try {
    body();
    last_error.clear();
    return 0;
  } catch (hresult_error const& error) {
    last_error = to_string(error.message());
    return error.code().value;
  } catch (std::exception const& error) {
    last_error = error.what();
    return E_FAIL;
  } catch (...) {
    last_error = "unknown native exception";
    return E_FAIL;
  }
}

template <typename T, typename F>
T* create(F&& body) noexcept {
  try {
    auto result = body();
    last_error.clear();
    return result;
  } catch (hresult_error const& error) {
    last_error = to_string(error.message());
  } catch (std::exception const& error) {
    last_error = error.what();
  } catch (...) {
    last_error = "unknown native exception";
  }
  return nullptr;
}

struct Callback {
  const void* data{};
  zpd_winui3_release_fn release{};

  Callback() = default;
  Callback(const void* data, zpd_winui3_release_fn release) : data(data), release(release) {}
  Callback(Callback const&) = delete;
  Callback& operator=(Callback const&) = delete;
  Callback(Callback&& other) noexcept : data(std::exchange(other.data, nullptr)), release(std::exchange(other.release, nullptr)) {}
  Callback& operator=(Callback&& other) noexcept {
    if (this != &other) {
      reset();
      data = std::exchange(other.data, nullptr);
      release = std::exchange(other.release, nullptr);
    }
    return *this;
  }
  ~Callback() { reset(); }
  void reset() {
    if (data && release) release(data);
    data = nullptr;
    release = nullptr;
  }
};

struct zpd_winui3_dispatcher {
  DispatcherQueue queue{nullptr};
};

struct zpd_winui3_app_context {
  DispatcherQueue queue{nullptr};
};

struct zpd_winui3_element {
  FrameworkElement value{nullptr};
  explicit zpd_winui3_element(FrameworkElement const& value) : value(value) {}
  virtual ~zpd_winui3_element() = default;
};

struct ButtonElement final : zpd_winui3_element {
  Button button{nullptr};
  event_token click_token{};
  Callback callback;
  explicit ButtonElement(Button const& value) : zpd_winui3_element(value), button(value) {}
  ~ButtonElement() override {
    if (click_token.value) button.Click(click_token);
  }
  void clear() {
    if (click_token.value) {
      button.Click(click_token);
      click_token = {};
    }
    callback.reset();
  }
};

struct TextBoxElement final : zpd_winui3_element {
  TextBox text_box{nullptr};
  event_token change_token{};
  Callback callback;
  explicit TextBoxElement(TextBox const& value) : zpd_winui3_element(value), text_box(value) {}
  ~TextBoxElement() override {
    if (change_token.value) text_box.TextChanged(change_token);
  }
  void clear() {
    if (change_token.value) {
      text_box.TextChanged(change_token);
      change_token = {};
    }
    callback.reset();
  }
};

struct zpd_winui3_window {
  Window window{nullptr};
  Grid shell{nullptr};
  MenuBar menu_bar{nullptr};
  FrameworkElement content{nullptr};
  Callback menu_callback;
  zpd_winui3_string_fn menu_invoke{};

  zpd_winui3_window() {
    window = Window();
    shell = Grid();
    menu_bar = MenuBar();

    RowDefinition menu_row;
    menu_row.Height(GridLengthHelper::Auto());
    RowDefinition content_row;
    content_row.Height(GridLengthHelper::FromValueAndType(1.0, GridUnitType::Star));
    shell.RowDefinitions().Append(menu_row);
    shell.RowDefinitions().Append(content_row);
    shell.Children().Append(menu_bar);
    menu_bar.Visibility(Visibility::Collapsed);
    window.Content(shell);
  }
};

struct LaunchState {
  const void* data;
  zpd_winui3_launch_fn launch;
};

LaunchState* launch_state{};

struct NativeApplication : ApplicationT<NativeApplication, IXamlMetadataProvider> {
  std::vector<IXamlMetadataProvider> providers;

  NativeApplication() {
    Hosting::WindowsXamlManager::InitializeForCurrentThread();
    Resources().MergedDictionaries().Append(XamlControlsResources());
  }

  void OnLaunched(LaunchActivatedEventArgs const&) {
    if (!launch_state || !launch_state->launch) return;
    zpd_winui3_app_context context{DispatcherQueue::GetForCurrentThread()};
    launch_state->launch(&context, launch_state->data);
  }

  std::vector<IXamlMetadataProvider> const& Providers() {
    if (providers.empty()) providers.push_back(XamlTypeInfo::XamlControlsXamlMetaDataProvider());
    return providers;
  }

  IXamlType GetXamlType(Windows::UI::Xaml::Interop::TypeName const& type) {
    for (auto const& provider : Providers()) if (auto result = provider.GetXamlType(type)) return result;
    return nullptr;
  }

  IXamlType GetXamlType(hstring const& name) {
    for (auto const& provider : Providers()) if (auto result = provider.GetXamlType(name)) return result;
    return nullptr;
  }

  com_array<XmlnsDefinition> GetXmlnsDefinitions() {
    std::vector<XmlnsDefinition> definitions;
    for (auto const& provider : Providers()) {
      auto values = provider.GetXmlnsDefinitions();
      definitions.insert(definitions.end(), values.begin(), values.end());
    }
    return {definitions.begin(), definitions.end()};
  }
};

Windows::System::VirtualKey key_from(zpd_winui3_string key) {
  auto value = bytes(key);
  if (value.size() == 1) {
    char character = static_cast<char>(std::toupper(static_cast<unsigned char>(value[0])));
    if (character >= 'A' && character <= 'Z') return static_cast<Windows::System::VirtualKey>(character);
    if (character >= '0' && character <= '9') return static_cast<Windows::System::VirtualKey>(character);
  }
  if (value == "enter") return Windows::System::VirtualKey::Enter;
  if (value == "escape") return Windows::System::VirtualKey::Escape;
  if (value == "delete") return Windows::System::VirtualKey::Delete;
  if (value == "space") return Windows::System::VirtualKey::Space;
  throw hresult_invalid_argument(L"unsupported keyboard accelerator key");
}

Windows::System::VirtualKeyModifiers modifiers_from(uint32_t value) {
  auto result = Windows::System::VirtualKeyModifiers::None;
  if (value & 1) result = result | Windows::System::VirtualKeyModifiers::Control;
  if (value & 2) result = result | Windows::System::VirtualKeyModifiers::Menu;
  if (value & 4) result = result | Windows::System::VirtualKeyModifiers::Shift;
  if (value & 8) result = result | Windows::System::VirtualKeyModifiers::Windows;
  return result;
}

void configure_accelerator(MenuFlyoutItem const& item, zpd_winui3_menu_item const& source) {
  if (!source.key.length) return;
  Microsoft::UI::Xaml::Input::KeyboardAccelerator accelerator;
  accelerator.Key(key_from(source.key));
  accelerator.Modifiers(modifiers_from(source.modifiers));
  item.KeyboardAccelerators().Append(accelerator);
}

void append_menu_items(
    Windows::Foundation::Collections::IVector<MenuFlyoutItemBase> const& target,
    zpd_winui3_window* window,
    const zpd_winui3_menu_item* items,
    size_t length) {
  for (size_t index = 0; index < length; ++index) {
    auto const& source = items[index];
    if (source.kind == ZPD_WINUI3_MENU_SEPARATOR) {
      target.Append(MenuFlyoutSeparator());
      continue;
    }
    if (source.kind == ZPD_WINUI3_MENU_SUBMENU) {
      MenuFlyoutSubItem submenu;
      submenu.Text(string_from(source.title));
      submenu.IsEnabled(source.enabled);
      append_menu_items(submenu.Items(), window, source.children, source.children_length);
      target.Append(submenu);
      continue;
    }
    MenuFlyoutItem item;
    item.Text(string_from(source.title));
    item.IsEnabled(source.enabled);
    configure_accelerator(item, source);
    auto id = std::string(bytes(source.id));
    item.Click([window, id = std::move(id)](Windows::Foundation::IInspectable const&, RoutedEventArgs const&) {
      if (!window->menu_invoke || !window->menu_callback.data) return;
      zpd_winui3_string value{reinterpret_cast<uint8_t const*>(id.data()), id.size()};
      window->menu_invoke(window->menu_callback.data, value);
    });
    target.Append(item);
  }
}

GridLength grid_length(zpd_winui3_grid_length value) {
  switch (value.kind) {
    case 0: return GridLengthHelper::Auto();
    case 1: return GridLengthHelper::FromPixels(value.value);
    case 2: return GridLengthHelper::FromValueAndType(value.value, GridUnitType::Star);
    default: throw hresult_invalid_argument(L"invalid GridLength kind");
  }
}

extern "C" {

int32_t zpd_winui3_application_run(const void* data, zpd_winui3_launch_fn launch, zpd_winui3_release_fn release) {
  bool released{};
  int32_t result = status([&] {
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    PACKAGE_VERSION minimum{};
    check_hresult(MddBootstrapInitialize2(0x00010008, L"", minimum, MddBootstrapInitializeOptions_OnNoMatch_ShowUI));
    bool apartment_initialized{};
    try {
      init_apartment(apartment_type::single_threaded);
      apartment_initialized = true;
      LaunchState state{data, launch};
      launch_state = &state;
      Application::Start([](auto&&) { make<NativeApplication>(); });
      launch_state = nullptr;
      if (data && release) {
        release(data);
        released = true;
      }
      if (apartment_initialized) uninit_apartment();
    } catch (...) {
      launch_state = nullptr;
      if (data && release) {
        release(data);
        released = true;
      }
      if (apartment_initialized) uninit_apartment();
      MddBootstrapShutdown();
      throw;
    }
    MddBootstrapShutdown();
  });
  if (!released && data && release) release(data);
  return result;
}

zpd_winui3_dispatcher* zpd_winui3_app_dispatcher(const zpd_winui3_app_context* context) {
  if (!context) return nullptr;
  return create<zpd_winui3_dispatcher>([&] { return new zpd_winui3_dispatcher{context->queue}; });
}

zpd_winui3_window* zpd_winui3_window_create(const zpd_winui3_app_context* context) {
  if (!context) return nullptr;
  return create<zpd_winui3_window>([] { return new zpd_winui3_window(); });
}

zpd_winui3_dispatcher* zpd_winui3_dispatcher_clone(const zpd_winui3_dispatcher* dispatcher) {
  if (!dispatcher) return nullptr;
  return create<zpd_winui3_dispatcher>([&] { return new zpd_winui3_dispatcher{dispatcher->queue}; });
}

void zpd_winui3_dispatcher_release(zpd_winui3_dispatcher* dispatcher) { delete dispatcher; }

bool zpd_winui3_dispatcher_try_enqueue(const zpd_winui3_dispatcher* dispatcher, int32_t priority, const void* data, zpd_winui3_invoke_fn invoke, zpd_winui3_release_fn release) {
  if (!dispatcher || !invoke) { if (data && release) release(data); return false; }
  auto callback = std::make_shared<Callback>(data, release);
  try {
    auto native_priority = static_cast<DispatcherQueuePriority>(priority);
    return dispatcher->queue.TryEnqueue(native_priority, [callback, invoke] { invoke(callback->data); });
  } catch (...) {
    return false;
  }
}

void zpd_winui3_window_release(zpd_winui3_window* window) {
  if (!window) return;
  try {
    window->menu_bar.Items().Clear();
    window->menu_callback.reset();
    window->window.Close();
  } catch (...) {
  }
  delete window;
}
int32_t zpd_winui3_window_set_title(zpd_winui3_window* window, zpd_winui3_string title) { return status([&] { window->window.Title(string_from(title)); }); }
int32_t zpd_winui3_window_resize(zpd_winui3_window* window, int32_t width, int32_t height) { return status([&] { window->window.AppWindow().Resize({width, height}); }); }
int32_t zpd_winui3_window_activate(zpd_winui3_window* window) { return status([&] { window->window.Activate(); }); }
int32_t zpd_winui3_window_close(zpd_winui3_window* window) { return status([&] { window->window.Close(); }); }
int32_t zpd_winui3_window_extend_content_into_title_bar(zpd_winui3_window* window, bool enabled) { return status([&] { window->window.ExtendsContentIntoTitleBar(enabled); }); }
int32_t zpd_winui3_window_set_title_bar(zpd_winui3_window* window, const zpd_winui3_element* element) { return status([&] { window->window.SetTitleBar(element ? element->value : nullptr); }); }

int32_t zpd_winui3_window_set_content(zpd_winui3_window* window, const zpd_winui3_element* element) {
  return status([&] {
    if (window->content) {
      uint32_t index{};
      if (window->shell.Children().IndexOf(window->content, index)) window->shell.Children().RemoveAt(index);
    }
    window->content = element ? element->value : nullptr;
    if (window->content) {
      Grid::SetRow(window->content, 1);
      window->shell.Children().Append(window->content);
    }
  });
}

int32_t zpd_winui3_window_set_backdrop(zpd_winui3_window* window, int32_t backdrop) {
  return status([&] {
    if (backdrop == 0) window->window.SystemBackdrop(nullptr);
    else if (backdrop == 1) window->window.SystemBackdrop(Media::MicaBackdrop());
    else if (backdrop == 2) window->window.SystemBackdrop(Media::DesktopAcrylicBackdrop());
    else throw hresult_invalid_argument(L"invalid SystemBackdrop value");
  });
}

int32_t zpd_winui3_window_clear_menu_bar(zpd_winui3_window* window) {
  return status([&] {
    window->menu_bar.Items().Clear();
    window->menu_bar.Visibility(Visibility::Collapsed);
    window->menu_invoke = nullptr;
    window->menu_callback.reset();
  });
}

int32_t zpd_winui3_window_set_menu_bar(zpd_winui3_window* window, const zpd_winui3_menu* menus, size_t length, const void* data, zpd_winui3_string_fn invoke, zpd_winui3_release_fn release) {
  return status([&] {
    window->menu_bar.Items().Clear();
    window->menu_callback = Callback(data, release);
    window->menu_invoke = invoke;
    for (size_t index = 0; index < length; ++index) {
      MenuBarItem menu;
      menu.Title(string_from(menus[index].title));
      append_menu_items(menu.Items(), window, menus[index].items, menus[index].items_length);
      window->menu_bar.Items().Append(menu);
    }
    window->menu_bar.Visibility(length ? Visibility::Visible : Visibility::Collapsed);
  });
}

zpd_winui3_element* zpd_winui3_button_create(zpd_winui3_string title) { return create<zpd_winui3_element>([&] { Button button; button.Content(box_value(string_from(title))); return new ButtonElement(button); }); }
int32_t zpd_winui3_button_set_title(zpd_winui3_element* element, zpd_winui3_string title) { return status([&] { static_cast<ButtonElement*>(element)->button.Content(box_value(string_from(title))); }); }
int32_t zpd_winui3_button_set_enabled(zpd_winui3_element* element, bool enabled) { return status([&] { static_cast<ButtonElement*>(element)->button.IsEnabled(enabled); }); }
int32_t zpd_winui3_button_clear_click_handler(zpd_winui3_element* element) { return status([&] { static_cast<ButtonElement*>(element)->clear(); }); }
int32_t zpd_winui3_button_set_click_handler(zpd_winui3_element* element, const void* data, zpd_winui3_invoke_fn invoke, zpd_winui3_release_fn release) {
  return status([&] { auto button = static_cast<ButtonElement*>(element); button->clear(); button->callback = Callback(data, release); button->click_token = button->button.Click([button, invoke](auto&&, auto&&) { invoke(button->callback.data); }); });
}

zpd_winui3_element* zpd_winui3_text_create(zpd_winui3_string text) { return create<zpd_winui3_element>([&] { TextBlock value; value.Text(string_from(text)); return new zpd_winui3_element(value); }); }
int32_t zpd_winui3_text_set_text(zpd_winui3_element* element, zpd_winui3_string value) { return status([&] { element->value.as<TextBlock>().Text(string_from(value)); }); }
int32_t zpd_winui3_text_set_wrapping(zpd_winui3_element* element, bool enabled) { return status([&] { element->value.as<TextBlock>().TextWrapping(enabled ? TextWrapping::Wrap : TextWrapping::NoWrap); }); }
int32_t zpd_winui3_text_set_alignment(zpd_winui3_element* element, int32_t alignment) { return status([&] { element->value.as<TextBlock>().TextAlignment(static_cast<TextAlignment>(alignment)); }); }

zpd_winui3_element* zpd_winui3_text_field_create(zpd_winui3_string value) { return create<zpd_winui3_element>([&] { TextBox box; box.Text(string_from(value)); return new TextBoxElement(box); }); }
int32_t zpd_winui3_text_field_set_value(zpd_winui3_element* element, zpd_winui3_string value) { return status([&] { static_cast<TextBoxElement*>(element)->text_box.Text(string_from(value)); }); }
int32_t zpd_winui3_text_field_get_value(const zpd_winui3_element* element, const void* data, zpd_winui3_string_fn receive) { return status([&] { auto utf8 = to_string(static_cast<TextBoxElement const*>(element)->text_box.Text()); zpd_winui3_string value{reinterpret_cast<uint8_t const*>(utf8.data()), utf8.size()}; receive(data, value); }); }
int32_t zpd_winui3_text_field_set_placeholder(zpd_winui3_element* element, zpd_winui3_string value) { return status([&] { static_cast<TextBoxElement*>(element)->text_box.PlaceholderText(string_from(value)); }); }
int32_t zpd_winui3_text_field_set_read_only(zpd_winui3_element* element, bool read_only) { return status([&] { static_cast<TextBoxElement*>(element)->text_box.IsReadOnly(read_only); }); }
int32_t zpd_winui3_text_field_clear_change_handler(zpd_winui3_element* element) { return status([&] { static_cast<TextBoxElement*>(element)->clear(); }); }
int32_t zpd_winui3_text_field_set_change_handler(zpd_winui3_element* element, const void* data, zpd_winui3_string_fn invoke, zpd_winui3_release_fn release) {
  return status([&] { auto box = static_cast<TextBoxElement*>(element); box->clear(); box->callback = Callback(data, release); box->change_token = box->text_box.TextChanged([box, invoke](auto&&, auto&&) { auto utf8 = to_string(box->text_box.Text()); zpd_winui3_string value{reinterpret_cast<uint8_t const*>(utf8.data()), utf8.size()}; invoke(box->callback.data, value); }); });
}

zpd_winui3_element* zpd_winui3_stack_panel_create(int32_t orientation, double spacing) { return create<zpd_winui3_element>([&] { StackPanel panel; panel.Orientation(static_cast<Orientation>(orientation)); panel.Spacing(spacing); return new zpd_winui3_element(panel); }); }
int32_t zpd_winui3_panel_append(zpd_winui3_element* panel, const zpd_winui3_element* child) { return status([&] { panel->value.as<Panel>().Children().Append(child->value); }); }
int32_t zpd_winui3_panel_clear(zpd_winui3_element* panel) { return status([&] { panel->value.as<Panel>().Children().Clear(); }); }

zpd_winui3_element* zpd_winui3_grid_create() { return create<zpd_winui3_element>([] { return new zpd_winui3_element(Grid()); }); }
int32_t zpd_winui3_grid_set_rows(zpd_winui3_element* element, const zpd_winui3_grid_length* rows, size_t length) { return status([&] { auto grid = element->value.as<Grid>(); grid.RowDefinitions().Clear(); for (size_t i = 0; i < length; ++i) { RowDefinition row; row.Height(grid_length(rows[i])); grid.RowDefinitions().Append(row); } }); }
int32_t zpd_winui3_grid_set_columns(zpd_winui3_element* element, const zpd_winui3_grid_length* columns, size_t length) { return status([&] { auto grid = element->value.as<Grid>(); grid.ColumnDefinitions().Clear(); for (size_t i = 0; i < length; ++i) { ColumnDefinition column; column.Width(grid_length(columns[i])); grid.ColumnDefinitions().Append(column); } }); }
int32_t zpd_winui3_grid_add(zpd_winui3_element* element, const zpd_winui3_element* child, int32_t row, int32_t column, int32_t row_span, int32_t column_span) { return status([&] { auto grid = element->value.as<Grid>(); Grid::SetRow(child->value, row); Grid::SetColumn(child->value, column); Grid::SetRowSpan(child->value, std::max(row_span, 1)); Grid::SetColumnSpan(child->value, std::max(column_span, 1)); grid.Children().Append(child->value); }); }

void zpd_winui3_element_release(zpd_winui3_element* element) { delete element; }
int32_t zpd_winui3_element_set_margin(zpd_winui3_element* element, zpd_winui3_thickness margin) { return status([&] { element->value.Margin({margin.left, margin.top, margin.right, margin.bottom}); }); }
int32_t zpd_winui3_element_set_width(zpd_winui3_element* element, double width) { return status([&] { element->value.Width(width); }); }
int32_t zpd_winui3_element_set_height(zpd_winui3_element* element, double height) { return status([&] { element->value.Height(height); }); }
int32_t zpd_winui3_element_set_horizontal_alignment(zpd_winui3_element* element, int32_t alignment) { return status([&] { element->value.HorizontalAlignment(static_cast<HorizontalAlignment>(alignment)); }); }
int32_t zpd_winui3_element_set_vertical_alignment(zpd_winui3_element* element, int32_t alignment) { return status([&] { element->value.VerticalAlignment(static_cast<VerticalAlignment>(alignment)); }); }

} // extern "C"
