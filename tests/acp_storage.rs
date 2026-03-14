use orbitshell::ui::Workspace;

#[test]
fn library_exports_workspace_type() {
    let _ = std::any::type_name::<Workspace>();
}
