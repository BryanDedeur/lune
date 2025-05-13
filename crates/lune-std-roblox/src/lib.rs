#![allow(clippy::cargo_common_metadata)]

use std::sync::OnceLock;

use mlua::prelude::*;
use mlua_luau_scheduler::LuaSpawnExt;

use lune_roblox::{
    document::{Document, DocumentError, DocumentFormat, DocumentKind},
    instance::{registry::InstanceRegistry, Instance},
    reflection::Database as ReflectionDatabase,
};

static REFLECTION_DATABASE: OnceLock<ReflectionDatabase> = OnceLock::new();

use lune_utils::TableBuilder;
use roblox_install::RobloxStudio;

const TYPEDEFS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/types.d.luau"));

/**
    Returns a string containing type definitions for the `roblox` standard library.
*/
#[must_use]
pub fn typedefs() -> String {
    TYPEDEFS.to_string()
}

/**
    Creates the `roblox` standard library module.

    # Errors

    Errors when out of memory.
*/
pub fn module(lua: Lua) -> LuaResult<LuaTable> {
    let mut roblox_constants = Vec::new();

    let roblox_module = lune_roblox::module(lua.clone())?;
    for pair in roblox_module.pairs::<LuaValue, LuaValue>() {
        roblox_constants.push(pair?);
    }

    TableBuilder::new(lua)?
        .with_values(roblox_constants)?
        .with_async_function("deserializePlace", deserialize_place)?
        .with_async_function("deserializeModel", deserialize_model)?
        .with_async_function("serializePlace", serialize_place)?
        .with_async_function("serializeModel", serialize_model)?
        .with_function("getAuthCookie", get_auth_cookie)?
        .with_function("getReflectionDatabase", get_reflection_database)?
        .with_function("implementProperty", implement_property)?
        .with_function("implementMethod", implement_method)?
        .with_function("studioApplicationPath", studio_application_path)?
        .with_function("studioContentPath", studio_content_path)?
        .with_function("studioPluginPath", studio_plugin_path)?
        .with_function("studioBuiltinPluginPath", studio_builtin_plugin_path)?
        .with_async_function("placeToMarkdown", place_to_markdown)?
        .build_readonly()
}

async fn deserialize_place(lua: Lua, contents: LuaString) -> LuaResult<LuaValue> {
    let bytes = contents.as_bytes().to_vec();
    let fut = lua.spawn_blocking(move || {
        let doc = Document::from_bytes(bytes, DocumentKind::Place)?;
        let data_model = doc.into_data_model_instance()?;
        Ok::<_, DocumentError>(data_model)
    });
    fut.await.into_lua_err()?.into_lua(&lua)
}

async fn deserialize_model(lua: Lua, contents: LuaString) -> LuaResult<LuaValue> {
    let bytes = contents.as_bytes().to_vec();
    let fut = lua.spawn_blocking(move || {
        let doc = Document::from_bytes(bytes, DocumentKind::Model)?;
        let instance_array = doc.into_instance_array()?;
        Ok::<_, DocumentError>(instance_array)
    });
    fut.await.into_lua_err()?.into_lua(&lua)
}

async fn serialize_place(
    lua: Lua,
    (data_model, as_xml): (LuaUserDataRef<Instance>, Option<bool>),
) -> LuaResult<LuaString> {
    let data_model = *data_model;
    let fut = lua.spawn_blocking(move || {
        let doc = Document::from_data_model_instance(data_model)?;
        let bytes = doc.to_bytes_with_format(match as_xml {
            Some(true) => DocumentFormat::Xml,
            _ => DocumentFormat::Binary,
        })?;
        Ok::<_, DocumentError>(bytes)
    });
    let bytes = fut.await.into_lua_err()?;
    lua.create_string(bytes)
}

async fn serialize_model(
    lua: Lua,
    (instances, as_xml): (Vec<LuaUserDataRef<Instance>>, Option<bool>),
) -> LuaResult<LuaString> {
    let instances = instances.iter().map(|i| **i).collect();
    let fut = lua.spawn_blocking(move || {
        let doc = Document::from_instance_array(instances)?;
        let bytes = doc.to_bytes_with_format(match as_xml {
            Some(true) => DocumentFormat::Xml,
            _ => DocumentFormat::Binary,
        })?;
        Ok::<_, DocumentError>(bytes)
    });
    let bytes = fut.await.into_lua_err()?;
    lua.create_string(bytes)
}

async fn place_to_markdown(lua: Lua, contents: LuaString) -> LuaResult<LuaString> {
    let bytes = contents.as_bytes().to_vec();
    let fut = lua.spawn_blocking(move || {
        let doc = Document::from_bytes(bytes, DocumentKind::Place)?;
        let data_model = doc.into_data_model_instance()?;
        let markdown = instance_to_markdown(data_model, 0);
        Ok::<_, DocumentError>(markdown)
    });
    
    let markdown = fut.await.into_lua_err()?;
    lua.create_string(&markdown)
}

fn instance_to_markdown(instance: Instance, depth: usize) -> String {
    // Get the full path of the instance
    let path = get_instance_path(&instance);
    
    // Get properties including UniqueId if available
    let props = get_all_properties(&instance);
    
    // Try to find UniqueId in properties for use in identifier
    let unique_id = props.iter()
        .find(|(name, _)| name == "UniqueId")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| {
            // Fallback to class name and instance name as identifier
            let class_name = instance.get_class_name();
            let name = instance.get_name();
            format!("{}-{}", class_name, name)
        });
    
    let mut result = format!("{} ({})\n", path, unique_id);
    
    // Add properties section
    if !props.is_empty() {
        for (prop_name, prop_value) in props {
            // Check if the property value contains multiple parts that need to be split
            if should_split_property(&prop_value) {
                result.push_str(&format!("- {}: \n", prop_name));
                
                // Format complex property values as nested items
                format_complex_property(&mut result, &prop_value);
            } else {
                result.push_str(&format!("- {}: {}\n", prop_name, prop_value));
            }
        }
    }
    
    // Add blank line
    result.push('\n');
    
    // Process children
    let children = instance.get_children();
    for child in children {
        result.push_str(&instance_to_markdown(child, depth + 1));
    }
    
    result
}

// Helper function to determine if a property value should be split into multiple lines
fn should_split_property(value: &str) -> bool {
    // Check for complex types
    value.contains("CFrame") || 
    value.contains("Vector3") || 
    value.contains("Vector2") ||
    value.contains("Color3") ||
    (value.contains('[') && (value.contains(',') || value.contains(';')))
}

// Helper function to format complex property values as nested items
fn format_complex_property(result: &mut String, value: &str) {
    if value.contains("CFrame") {
        // Format CFrame properties
        result.push_str("  - position: \n");
        
        // Extract position
        if let Some(pos) = value.find("position: Vector3") {
            let pos_start = pos + "position: Vector3".len();
            let pos_end = value[pos_start..].find('}').unwrap_or(value.len() - pos_start) + pos_start;
            let pos_str = &value[pos_start..pos_end];
            
            // Format position components
            if let Some(x) = pos_str.find("x: ") {
                let x_start = x + 3;
                let x_end = pos_str[x_start..].find(',').unwrap_or(pos_str.len() - x_start) + x_start;
                result.push_str(&format!("    - x: {}\n", &pos_str[x_start..x_end]));
            }
            
            if let Some(y) = pos_str.find("y: ") {
                let y_start = y + 3;
                let y_end = pos_str[y_start..].find(',').unwrap_or(pos_str.len() - y_start) + y_start;
                result.push_str(&format!("    - y: {}\n", &pos_str[y_start..y_end]));
            }
            
            if let Some(z) = pos_str.find("z: ") {
                let z_start = z + 3;
                let z_end = pos_str[z_start..].find(',').unwrap_or(pos_str.len() - z_start) + z_start;
                result.push_str(&format!("    - z: {}\n", &pos_str[z_start..z_end]));
            }
        }
        
        result.push_str("  - orientation: \n");
        
        // Extract orientation vectors in a cleaner way
        if let Some(orientation) = value.find("orientation: Matrix3") {
            let ori_start = orientation + "orientation: Matrix3".len();
            
            // Extract x axis
            if let Some(x_axis) = value[ori_start..].find("x: Vector3") {
                let x_vec_start = ori_start + x_axis + "x: Vector3".len();
                if let Some(x_end) = value[x_vec_start..].find('}') {
                    let x_vec = parse_vector_components(&value[x_vec_start..x_vec_start+x_end]);
                    result.push_str(&format!("    - x-axis: [{}, {}, {}]\n", x_vec.0, x_vec.1, x_vec.2));
                }
            }
            
            // Extract y axis
            if let Some(y_axis) = value[ori_start..].find("y: Vector3") {
                let y_vec_start = ori_start + y_axis + "y: Vector3".len();
                if let Some(y_end) = value[y_vec_start..].find('}') {
                    let y_vec = parse_vector_components(&value[y_vec_start..y_vec_start+y_end]);
                    result.push_str(&format!("    - y-axis: [{}, {}, {}]\n", y_vec.0, y_vec.1, y_vec.2));
                }
            }
            
            // Extract z axis
            if let Some(z_axis) = value[ori_start..].find("z: Vector3") {
                let z_vec_start = ori_start + z_axis + "z: Vector3".len();
                if let Some(z_end) = value[z_vec_start..].find('}') {
                    let z_vec = parse_vector_components(&value[z_vec_start..z_vec_start+z_end]);
                    result.push_str(&format!("    - z-axis: [{}, {}, {}]\n", z_vec.0, z_vec.1, z_vec.2));
                }
            }
        }
    } else if value.contains("Vector3") {
        // Extract Vector3 components
        if let Some(vec_start) = value.find('{') {
            let vec_end = value[vec_start..].find('}').unwrap_or(value.len() - vec_start) + vec_start;
            let vec_str = &value[vec_start..vec_end];
            
            // Format Vector3 components
            if let Some(x) = vec_str.find("x: ") {
                let x_start = x + 3;
                let x_end = vec_str[x_start..].find(',').unwrap_or(vec_str.len() - x_start) + x_start;
                result.push_str(&format!("  - x: {}\n", &vec_str[x_start..x_end]));
            }
            
            if let Some(y) = vec_str.find("y: ") {
                let y_start = y + 3;
                let y_end = vec_str[y_start..].find(',').unwrap_or(vec_str.len() - y_start) + y_start;
                result.push_str(&format!("  - y: {}\n", &vec_str[y_start..y_end]));
            }
            
            if let Some(z) = vec_str.find("z: ") {
                let z_start = z + 3;
                let z_end = vec_str[z_start..].find(',').unwrap_or(vec_str.len() - z_start) + z_start;
                result.push_str(&format!("  - z: {}\n", &vec_str[z_start..z_end]));
            }
        }
    } else if value.contains("Color3") {
        // Extract Color3 components
        if let Some(col_start) = value.find('{') {
            let col_end = value[col_start..].find('}').unwrap_or(value.len() - col_start) + col_start;
            let col_str = &value[col_start..col_end];
            
            // Format Color3 components
            if let Some(r) = col_str.find("r: ") {
                let r_start = r + 3;
                let r_end = col_str[r_start..].find(',').unwrap_or(col_str.len() - r_start) + r_start;
                result.push_str(&format!("  - r: {}\n", &col_str[r_start..r_end]));
            }
            
            if let Some(g) = col_str.find("g: ") {
                let g_start = g + 3;
                let g_end = col_str[g_start..].find(',').unwrap_or(col_str.len() - g_start) + g_start;
                result.push_str(&format!("  - g: {}\n", &col_str[g_start..g_end]));
            }
            
            if let Some(b) = col_str.find("b: ") {
                let b_start = b + 3;
                let b_end = col_str[b_start..].find(',').unwrap_or(col_str.len() - b_start) + b_start;
                result.push_str(&format!("  - b: {}\n", &col_str[b_start..b_end]));
            }
        }
    } else {
        // For any other complex types or arrays
        let stripped = value.trim_start_matches(|c| c == '[' || c == '{')
            .trim_end_matches(|c| c == ']' || c == '}');
        
        // Split by comma or semicolon
        let parts: Vec<&str> = stripped.split(|c| c == ',' || c == ';').collect();
        for part in parts {
            result.push_str(&format!("  - {}\n", part.trim()));
        }
    }
}

// Helper function to parse vector components
fn parse_vector_components(vec_str: &str) -> (f32, f32, f32) {
    let mut x = 0.0;
    let mut y = 0.0;
    let mut z = 0.0;
    
    if let Some(x_pos) = vec_str.find("x: ") {
        if let Some(x_end) = vec_str[x_pos+3..].find(',') {
            if let Ok(val) = vec_str[x_pos+3..x_pos+3+x_end].trim().parse::<f32>() {
                x = val;
            }
        }
    }
    
    if let Some(y_pos) = vec_str.find("y: ") {
        if let Some(y_end) = vec_str[y_pos+3..].find(',') {
            if let Ok(val) = vec_str[y_pos+3..y_pos+3+y_end].trim().parse::<f32>() {
                y = val;
            }
        }
    }
    
    if let Some(z_pos) = vec_str.find("z: ") {
        if let Some(z_end) = vec_str[z_pos+3..].find(',') {
            if let Ok(val) = vec_str[z_pos+3..z_pos+3+z_end].trim().parse::<f32>() {
                z = val;
            }
        } else {
            // Last component might not have a comma after it
            if let Ok(val) = vec_str[z_pos+3..].trim().parse::<f32>() {
                z = val;
            }
        }
    }
    
    (x, y, z)
}

// Helper function to get the full path of an instance
fn get_instance_path(instance: &Instance) -> String {
    let mut path_parts = Vec::new();
    path_parts.push(instance.get_name());
    
    let mut current = instance.get_parent();
    while let Some(parent) = current {
        path_parts.push(parent.get_name());
        current = parent.get_parent();
    }
    
    // Reverse and join with slashes
    path_parts.reverse();
    path_parts.join("/")
}

// Helper function to get properties as key-value pairs
fn get_all_properties(instance: &Instance) -> Vec<(String, String)> {
    let mut properties = Vec::new();
    
    // Add Class name
    properties.push(("ClassName".to_string(), instance.get_class_name().to_string()));
    
    // Add Name
    properties.push(("Name".to_string(), instance.get_name()));
    
    // Try to get the UniqueId property directly
    if let Some(unique_id) = instance.get_property("UniqueId") {
        // Format UniqueId in a cleaner way
        let formatted_id = format_unique_id(&format!("{:?}", unique_id));
        properties.push(("UniqueId".to_string(), formatted_id));
    }
    
    // We can't directly access all properties, but we can check common ones
    let properties_to_check = [
        "Position", "Size", "CFrame", "Anchored", "CanCollide",
        "Transparency", "Material", "Color", "Text", "Font", "TextSize",
        "BackgroundColor3", "BorderColor3", "BorderSizePixel", "ZIndex",
        "Enabled", "Visible", "Value", "Source", "MaxPlayers"
    ];
    
    for prop in properties_to_check {
        if let Some(value) = instance.get_property(prop) {
            let formatted_value = if prop == "Material" || prop.contains("State") || prop.contains("Type") {
                format_enum_value(&format!("{:?}", value))
            } else {
                format!("{:?}", value)
            };
            properties.push((prop.to_string(), formatted_value));
        }
    }
    
    // Add attributes if available
    let attributes = instance.get_attributes();
    for (key, value) in attributes {
        properties.push((format!("Attribute.{}", key), format!("{:?}", value)));
    }
    
    // Add tags
    let tags = instance.get_tags();
    if !tags.is_empty() {
        properties.push(("Tags".to_string(), format!("{:?}", tags)));
    }
    
    // Add parent
    if let Some(parent) = instance.get_parent() {
        properties.push(("Parent".to_string(), parent.get_name()));
    } else {
        properties.push(("Parent".to_string(), "nil".to_string()));
    }
    
    // Sort properties alphabetically by name
    properties.sort_by(|a, b| a.0.cmp(&b.0));
    
    properties
}

// Format a UniqueId string to be more readable
fn format_unique_id(raw_id: &str) -> String {
    // Extract the important parts from the debug format
    if let Some(start) = raw_id.find('{') {
        if let Some(end) = raw_id.rfind('}') {
            let content = &raw_id[start+1..end].trim();
            
            // Create a cleaner representation with just the hex value
            if let Some(random_idx) = content.find("random:") {
                if let Some(random_end) = content[random_idx..].find(',') {
                    let random_part = &content[random_idx+8..random_idx+random_end-1];
                    // Return just the random part as hex (usually the most useful part)
                    return format!("{}", random_part);
                }
            }
            
            // If we can't parse it as expected, return a cleaned-up version
            return content.replace(", ", "-").replace("random: ", "");
        }
    }
    
    // Fallback to the original if parsing fails
    raw_id.to_string()
}

// Format an Enum value to be more readable
fn format_enum_value(raw_enum: &str) -> String {
    // For Enum values, extract just the numeric value
    if let Some(value_start) = raw_enum.find("value: ") {
        if let Some(value_end) = raw_enum[value_start..].find('}') {
            let value = &raw_enum[value_start+7..value_start+value_end];
            return format!("{}", value.trim());
        }
    }
    
    // Fallback to the original if parsing fails
    raw_enum.to_string()
}

fn get_auth_cookie(_: &Lua, raw: Option<bool>) -> LuaResult<Option<String>> {
    if matches!(raw, Some(true)) {
        Ok(rbx_cookie::get_value())
    } else {
        Ok(rbx_cookie::get())
    }
}

fn get_reflection_database(_: &Lua, _: ()) -> LuaResult<ReflectionDatabase> {
    Ok(*REFLECTION_DATABASE.get_or_init(ReflectionDatabase::new))
}

fn implement_property(
    lua: &Lua,
    (class_name, property_name, property_getter, property_setter): (
        String,
        String,
        LuaFunction,
        Option<LuaFunction>,
    ),
) -> LuaResult<()> {
    let property_setter = if let Some(setter) = property_setter {
        setter
    } else {
        let property_name = property_name.clone();
        lua.create_function(move |_, _: LuaMultiValue| {
            Err::<(), _>(LuaError::runtime(format!(
                "Property '{property_name}' is read-only"
            )))
        })?
    };
    InstanceRegistry::insert_property_getter(lua, &class_name, &property_name, property_getter)
        .into_lua_err()?;
    InstanceRegistry::insert_property_setter(lua, &class_name, &property_name, property_setter)
        .into_lua_err()?;
    Ok(())
}

fn implement_method(
    lua: &Lua,
    (class_name, method_name, method): (String, String, LuaFunction),
) -> LuaResult<()> {
    InstanceRegistry::insert_method(lua, &class_name, &method_name, method).into_lua_err()?;
    Ok(())
}

fn studio_application_path(_: &Lua, _: ()) -> LuaResult<String> {
    RobloxStudio::locate()
        .map(|rs| rs.application_path().display().to_string())
        .map_err(LuaError::external)
}

fn studio_content_path(_: &Lua, _: ()) -> LuaResult<String> {
    RobloxStudio::locate()
        .map(|rs| rs.content_path().display().to_string())
        .map_err(LuaError::external)
}

fn studio_plugin_path(_: &Lua, _: ()) -> LuaResult<String> {
    RobloxStudio::locate()
        .map(|rs| rs.plugins_path().display().to_string())
        .map_err(LuaError::external)
}

fn studio_builtin_plugin_path(_: &Lua, _: ()) -> LuaResult<String> {
    RobloxStudio::locate()
        .map(|rs| rs.built_in_plugins_path().display().to_string())
        .map_err(LuaError::external)
}
