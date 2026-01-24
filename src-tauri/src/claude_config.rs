pub(crate) fn read_steer_enabled() -> Result<Option<bool>, String> {
    Ok(None)
}

pub(crate) fn read_collab_enabled() -> Result<Option<bool>, String> {
    Ok(None)
}

pub(crate) fn read_unified_exec_enabled() -> Result<Option<bool>, String> {
    Ok(None)
}

pub(crate) fn write_steer_enabled(_enabled: bool) -> Result<(), String> {
    Ok(())
}

pub(crate) fn write_collab_enabled(_enabled: bool) -> Result<(), String> {
    Ok(())
}

pub(crate) fn write_unified_exec_enabled(_enabled: bool) -> Result<(), String> {
    Ok(())
}
