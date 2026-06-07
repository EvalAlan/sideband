#include "my_application.h"

#include <flutter_linux/flutter_linux.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include "flutter/generated_plugin_registrant.h"

static GdkPixbuf* load_icon_from_path(const char* path, int size) {
  if (!path || !g_file_test(path, G_FILE_TEST_EXISTS)) return nullptr;
  g_autoptr(GError) error = nullptr;
  return gdk_pixbuf_new_from_file_at_scale(path, size, size, TRUE, &error);
}

static GdkPixbuf* load_window_icon(int size) {
  // Flutter bundles assets next to the executable at:
  //   <bundle>/data/flutter_assets/assets/icon_256x256.png
  // The previous relative cwd lookup silently failed when launched from a
  // desktop file/AppImage, leaving GTK to show the generic X icon.
  g_autofree gchar* exe = g_file_read_link("/proc/self/exe", nullptr);
  if (exe) {
    g_autofree gchar* exe_dir = g_path_get_dirname(exe);
    g_autofree gchar* bundled = g_build_filename(
        exe_dir, "data", "flutter_assets", "assets", "icon_256x256.png",
        nullptr);
    if (GdkPixbuf* icon = load_icon_from_path(bundled, size)) return icon;
  }

  // Fallback for unusual dev launches from the bundle directory.
  if (GdkPixbuf* icon =
          load_icon_from_path("data/flutter_assets/assets/icon_256x256.png", size)) {
    return icon;
  }

  // Final fallback for source-tree launches.
  return load_icon_from_path("assets/icon_256x256.png", size);
}

static void set_window_icon(GtkWindow* window) {
  gtk_window_set_default_icon_name("sideband_gui");
  gtk_window_set_icon_name(window, "sideband_gui");

  g_autoptr(GdkPixbuf) icon = load_window_icon(256);
  if (!icon) return;

  gtk_window_set_icon(window, icon);
  GList* icons = nullptr;
  icons = g_list_append(icons, gdk_pixbuf_copy(icon));
  g_autoptr(GdkPixbuf) small =
      gdk_pixbuf_scale_simple(icon, 64, 64, GDK_INTERP_BILINEAR);
  if (small) icons = g_list_append(icons, small);
  gtk_window_set_default_icon_list(icons);
  g_list_free_full(icons, g_object_unref);
}

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));

  gtk_window_set_title(window, "Sideband");
  gtk_window_set_default_size(window, 1280, 800);

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));

  set_window_icon(window);

  gtk_widget_grab_focus(GTK_WIDGET(view));
  gtk_widget_show(GTK_WIDGET(view));
  gtk_widget_show(GTK_WIDGET(window));
}

// Implements GApplication::local_command_line.
static gboolean my_application_local_command_line(GApplication* application,
                                                  gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  // Strip out the first argument as it is the binary name.
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }

  g_application_activate(application);
  *exit_status = 0;

  return TRUE;
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application startup.

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application shutdown.

  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line =
      my_application_local_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);

  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     G_APPLICATION_NON_UNIQUE, nullptr));
}
