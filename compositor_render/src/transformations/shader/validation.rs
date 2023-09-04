use naga::{ArraySize, Constant, ConstantInner, Handle, Module, ShaderStage, Type};

use super::VERTEX_ENTRYPOINT_NAME;

#[derive(Debug, thiserror::Error)]
pub enum ShaderValidationError {
    #[error("A global that should be declared in the shader is not declared: \n{0:#?}.")]
    GlobalNotFound(naga::GlobalVariable),

    #[error("A global in the shader has a wrong type. {0}")]
    GlobalBadType(#[source] TypeEquivalenceError),

    #[error("Could not find a vertex shader entrypoint. Expected \"fn {VERTEX_ENTRYPOINT_NAME}(input: VertexInput)\"")]
    VertexShaderNotFound,

    #[error("Wrong vertex shader argument amount: found {0}, expected 1.")]
    VertexShaderBadArgumentAmount(usize),

    // TODO: do we enforce type name from header?
    // #[error("The input type of the vertex shader has a name that cannot be found in the header.")]
    #[error("The input type of the vertex shader (\"{0}\") was not declared.")]
    VertexShaderBadInputTypeName(String),

    #[error("The vertex shader input has a wrong type. {0}")]
    VertexShaderBadInput(#[source] TypeEquivalenceError),
}

pub fn validate_contains_header(
    header: &naga::Module,
    shader: &naga::Module,
) -> Result<(), ShaderValidationError> {
    validate_globals(header, shader)?;
    validate_vertex_input(header, shader)?;
    Ok(())
}

fn validate_globals(
    header: &naga::Module,
    shader: &naga::Module,
) -> Result<(), ShaderValidationError> {
    for (_, global) in header.global_variables.iter() {
        let (_, global_in_shader) = shader
            .global_variables
            .iter()
            .find(|(_, s_global)| {
                s_global.space == global.space && s_global.binding == global.binding
            })
            .ok_or_else(|| ShaderValidationError::GlobalNotFound(global.clone()))?;

        validate_type_equivalent(global.ty, header, global_in_shader.ty, shader)
            .map_err(ShaderValidationError::GlobalBadType)?;
    }

    Ok(())
}

fn validate_vertex_input(
    header: &naga::Module,
    shader: &naga::Module,
) -> Result<(), ShaderValidationError> {
    let vertex = shader
        .entry_points
        .iter()
        .find(|entry_point| {
            entry_point.name == super::VERTEX_ENTRYPOINT_NAME
                && entry_point.stage == ShaderStage::Vertex
        })
        .ok_or(ShaderValidationError::VertexShaderNotFound)?;

    if vertex.function.arguments.len() != 1 {
        return Err(ShaderValidationError::VertexShaderBadArgumentAmount(
            vertex.function.arguments.len(),
        ));
    }

    let vertex_input = vertex.function.arguments[0].ty;
    let vertex_input_type = &shader.types[vertex_input];

    let (header_vertex_input, _) = header
        .types
        .iter()
        .find(|(_, ty)| ty.name == vertex_input_type.name)
        .ok_or_else(|| {
            ShaderValidationError::VertexShaderBadInputTypeName(
                vertex_input_type
                    .name
                    .clone()
                    .unwrap_or("<unknown>".to_string()),
            )
        })?;

    validate_type_equivalent(header_vertex_input, header, vertex_input, shader)
        .map_err(ShaderValidationError::VertexShaderBadInput)?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum TypeEquivalenceError {
    #[error("Type names don't match: {0:?} != {1:?}.")]
    TypeNameMismatch(Option<String>, Option<String>),

    #[error(
        "Type internal structure doesn't match:\nExpected:\n{expected:#?}\n\nActual:\n{actual:#?}."
    )]
    TypeStructureMismatch {
        expected: naga::TypeInner,
        actual: naga::TypeInner,
    },

    #[error("Sizes of an array don't match: {0:?} != {1:?}.")]
    ArraySizeMismatch(ArraySizeOrConstant, ArraySizeOrConstant),

    #[error("A composite type was used as an array length specifier. {0:?}")]
    // don't think this will ever happen
    CompositeTypeAsArrayLen(ConstantInner),
}

#[derive(Debug)]
pub enum ArraySizeOrConstant {
    ArraySize(ArraySize),
    Constant(Constant),
}

fn validate_type_equivalent(
    ty1: Handle<Type>,
    mod1: &Module,
    ty2: Handle<Type>,
    mod2: &Module,
) -> Result<(), TypeEquivalenceError> {
    let type1 = &mod1.types[ty1];
    let type2 = &mod2.types[ty2];

    if type1.name != type2.name {
        return Err(TypeEquivalenceError::TypeNameMismatch(
            type1.name.clone(),
            type2.name.clone(),
        ));
    }

    let ti1 = match type1.inner.canonical_form(&mod1.types) {
        Some(t) => t,
        None => type1.inner.clone(),
    };
    let ti2 = match type2.inner.canonical_form(&mod2.types) {
        Some(t) => t,
        None => type2.inner.clone(),
    };

    match ti1 {
        naga::TypeInner::Scalar { .. }
        | naga::TypeInner::Vector { .. }
        | naga::TypeInner::Matrix { .. }
        | naga::TypeInner::Atomic { .. }
        | naga::TypeInner::Image { .. }
        | naga::TypeInner::Sampler { .. }
        | naga::TypeInner::AccelerationStructure
        | naga::TypeInner::RayQuery
        | naga::TypeInner::ValuePointer { .. } => {
            if ti1 != ti2 {
                return Err(TypeEquivalenceError::TypeStructureMismatch {
                    expected: type1.inner.clone(),
                    actual: type2.inner.clone(),
                });
            }
        }

        naga::TypeInner::Array {
            base: base1,
            size: size1,
            stride: stride1,
        } => {
            let naga::TypeInner::Array {
                base: base2,
                size: size2,
                stride: stride2,
            } = ti2
            else {
                return Err(TypeEquivalenceError::TypeStructureMismatch {
                    expected: ti1,
                    actual: ti2,
                });
            };

            if stride1 != stride2 {
                return Err(TypeEquivalenceError::TypeStructureMismatch {
                    expected: ti1,
                    actual: ti2,
                });
            }

            validate_array_size_equivalent(size1, mod1, size2, mod2)?;
            return validate_type_equivalent(base1, mod1, base2, mod2);
        }

        naga::TypeInner::BindingArray {
            base: base1,
            size: size1,
        } => {
            let naga::TypeInner::BindingArray {
                base: base2,
                size: size2,
            } = ti2
            else {
                return Err(TypeEquivalenceError::TypeStructureMismatch {
                    expected: ti1,
                    actual: ti2,
                });
            };

            validate_array_size_equivalent(size1, mod1, size2, mod2)?;
            return validate_type_equivalent(base1, mod1, base2, mod2);
        }

        naga::TypeInner::Struct {
            members: ref members1,
            span: span1,
        } => {
            let naga::TypeInner::Struct {
                members: ref members2,
                span: span2,
            } = ti2
            else {
                return Err(TypeEquivalenceError::TypeStructureMismatch {
                    expected: ti1.clone(),
                    actual: ti2.clone(),
                });
            };

            if span1 != span2 || members1.len() != members2.len() {
                return Err(TypeEquivalenceError::TypeStructureMismatch {
                    expected: ti1.clone(),
                    actual: ti2.clone(),
                });
            }

            for (m1, m2) in members1.iter().zip(members2.iter()) {
                if m1.binding != m2.binding || m1.name != m2.name || m1.offset != m2.offset {
                    return Err(TypeEquivalenceError::TypeStructureMismatch {
                        expected: ti1,
                        actual: ti2,
                    });
                }

                validate_type_equivalent(m1.ty, mod1, m2.ty, mod2)?;
            }
        }

        naga::TypeInner::Pointer { .. } => {
            panic!("this should never happen bc of canonicalization")
        }
    }

    Ok(())
}

fn validate_array_size_equivalent(
    size1: ArraySize,
    mod1: &Module,
    size2: ArraySize,
    mod2: &Module,
) -> Result<(), TypeEquivalenceError> {
    match (size1, size2) {
        (ArraySize::Constant(_), ArraySize::Dynamic)
        | (ArraySize::Dynamic, ArraySize::Constant(_)) => {
            Err(TypeEquivalenceError::ArraySizeMismatch(
                ArraySizeOrConstant::ArraySize(size1),
                ArraySizeOrConstant::ArraySize(size2),
            ))
        }

        (ArraySize::Constant(c1), ArraySize::Constant(c2)) => {
            validate_constant_value_equivalent(c1, mod1, c2, mod2)
        }
        (ArraySize::Dynamic, ArraySize::Dynamic) => Ok(()),
    }
}

fn validate_constant_value_equivalent(
    c1: Handle<Constant>,
    mod1: &Module,
    c2: Handle<Constant>,
    mod2: &Module,
) -> Result<(), TypeEquivalenceError> {
    let ci1 = &mod1.constants[c1].inner;
    let ci2 = &mod2.constants[c2].inner;

    // TODO: what do we do with c1.specialization? It doesn't occur in WGSL, but it can occur in vulkan shaders, which we might want to support later.
    // There are also plans of adding them to WGSL

    if let ConstantInner::Composite { .. } = ci2 {
        return Err(TypeEquivalenceError::CompositeTypeAsArrayLen(ci2.clone()));
    }

    match (ci1, ci2) {
        (
            ConstantInner::Scalar {
                width: _, // what about this
                value: v1,
            },
            ConstantInner::Scalar {
                width: _,
                value: v2,
            },
        ) => match (v1, v2) {
            (naga::ScalarValue::Sint(a), naga::ScalarValue::Sint(b)) => {
                if a == b {
                    Ok(())
                } else {
                    Err(TypeEquivalenceError::ArraySizeMismatch(
                        ArraySizeOrConstant::Constant(mod1.constants[c1].clone()),
                        ArraySizeOrConstant::Constant(mod2.constants[c2].clone()),
                    ))
                }
            }

            (naga::ScalarValue::Uint(a), naga::ScalarValue::Uint(b)) => {
                if a == b {
                    Ok(())
                } else {
                    Err(TypeEquivalenceError::ArraySizeMismatch(
                        ArraySizeOrConstant::Constant(mod1.constants[c1].clone()),
                        ArraySizeOrConstant::Constant(mod2.constants[c2].clone()),
                    ))
                }
            }

            // don't really know whether this should be handled separately
            (naga::ScalarValue::Sint(a), naga::ScalarValue::Uint(b))
            | (naga::ScalarValue::Uint(b), naga::ScalarValue::Sint(a)) => {
                if *a as u64 == *b {
                    Ok(())
                } else {
                    Err(TypeEquivalenceError::ArraySizeMismatch(
                        ArraySizeOrConstant::Constant(mod1.constants[c1].clone()),
                        ArraySizeOrConstant::Constant(mod2.constants[c2].clone()),
                    ))
                }
            }

            _ => Err(TypeEquivalenceError::ArraySizeMismatch(
                ArraySizeOrConstant::Constant(mod1.constants[c1].clone()),
                ArraySizeOrConstant::Constant(mod2.constants[c2].clone()),
            )),
        },
        (ConstantInner::Composite { .. }, _) => {
            Err(TypeEquivalenceError::CompositeTypeAsArrayLen(ci1.clone()))
        }
        (_, ConstantInner::Composite { .. }) => {
            Err(TypeEquivalenceError::CompositeTypeAsArrayLen(ci2.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_len() {
        let s1 = r#"
        var a: array<i32, 16>;
        "#;

        let s2 = r#"
        var a: array<i32, 17>;
        "#;

        let s1 = naga::front::wgsl::parse_str(s1).unwrap();
        let s2 = naga::front::wgsl::parse_str(s2).unwrap();

        assert!(validate_contains_header(&s1, &s2).is_err());
    }

    #[test]
    fn binding() {
        let s1 = r#"
        @group(0) @binding(0) var a: i32;
        "#;

        let s2 = r#"
        @group(0) @binding(1) var a: i32;
        "#;

        let s1 = naga::front::wgsl::parse_str(s1).unwrap();
        let s2 = naga::front::wgsl::parse_str(s2).unwrap();

        assert!(validate_contains_header(&s1, &s2).is_err());
    }

    #[test]
    fn vertex_input() {
        let s1 = r#"
        struct VertexInput {
            @location(0) position: vec3<f32>,
            @location(1) tex_coords: vec2<f32>,
        }
        "#;

        let s2 = r#"
        struct VertexInput {
            @location(0) position: vec3<f32>,
            @location(1) tex_coords: vec2<u32>,
        }

        @vertex
        fn vs_main(in: VertexInput) -> @builtin(position) vec4<f32> {
            return vec4(0);
        }
        "#;

        let s1 = naga::front::wgsl::parse_str(s1).unwrap();
        let s2 = naga::front::wgsl::parse_str(s2).unwrap();

        assert!(validate_contains_header(&s1, &s2).is_err());
    }

    #[test]
    fn vertex_input_locations() {
        let s1 = r#"
        struct VertexInput {
            @location(0) position: vec3<f32>,
            @location(1) tex_coords: vec2<f32>,
        }
        "#;

        let s2 = r#"
        struct VertexInput {
            @location(0) position: vec3<f32>,
            @location(2) tex_coords: vec2<f32>,
        }

        @vertex
        fn vs_main(in: VertexInput) -> @builtin(position) vec4<f32> {
            return vec4(0);
        }
        "#;

        let s1 = naga::front::wgsl::parse_str(s1).unwrap();
        let s2 = naga::front::wgsl::parse_str(s2).unwrap();

        assert!(validate_contains_header(&s1, &s2).is_err());
    }
}