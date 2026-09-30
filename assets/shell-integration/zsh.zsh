# Kullanıcı başlangıç dosyalarını kendi dizinlerinden yükle; profillere yazma.
typeset -g __vyber_integration_dir=$ZDOTDIR
typeset -g __vyber_user_zdotdir=${VYBER_ORIGINAL_ZDOTDIR:-$HOME}
typeset -g __vyber_user_zdotdir_set=${+VYBER_ORIGINAL_ZDOTDIR}
unset VYBER_ORIGINAL_ZDOTDIR

function __vyber_restore_zdotdir() {
    if (( __vyber_user_zdotdir_set )); then
        ZDOTDIR=$__vyber_user_zdotdir
    else
        unset ZDOTDIR
    fi
}

function __vyber_remember_zdotdir() {
    __vyber_user_zdotdir=${ZDOTDIR:-$HOME}
    __vyber_user_zdotdir_set=${+ZDOTDIR}
}

function __vyber_report_cwd() {
    emulate -L zsh
    # UTF-8 ve boşluk içeren yolları OSC 7 için bayt düzeyinde kodla.
    local LC_ALL=C encoded_path='' character hex index
    for (( index = 1; index <= ${#PWD}; index++ )); do
        character=$PWD[index]
        case $character in
            [/._~A-Za-z0-9-]) encoded_path+=$character ;;
            *) printf -v hex '%02X' "'$character"; encoded_path+=%$hex ;;
        esac
    done
    printf '\e]7;file://localhost%s\a' "$encoded_path"
}

function __vyber_install_hooks() {
    # Mevcut istem kancalarını koru ve tekrar kayıt oluşturma.
    typeset -ga precmd_functions chpwd_functions
    (( ${precmd_functions[(Ie)__vyber_report_cwd]} )) || precmd_functions+=(__vyber_report_cwd)
    (( ${chpwd_functions[(Ie)__vyber_report_cwd]} )) || chpwd_functions+=(__vyber_report_cwd)
    __vyber_report_cwd
}

__vyber_restore_zdotdir
[[ -r ${ZDOTDIR:-$HOME}/.zshenv ]] && builtin source "${ZDOTDIR:-$HOME}/.zshenv"
__vyber_remember_zdotdir
ZDOTDIR=$__vyber_integration_dir
