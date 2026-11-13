import tseslint from 'typescript-eslint';
export default tseslint.config(...tseslint.configs.recommended,{ignores:['dist/**','node_modules/**','design/frames/**','target/**','apps/desktop/src-tauri/target/**']});
